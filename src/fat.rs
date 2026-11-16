//! MBR / GPT Partition Table and FAT12 / FAT16 / FAT32 Filesystem Reader.

extern crate alloc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::virtio::{self, VirtIoBlkDevice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

pub struct FatVolume {
    pub part_lba: u64,
    pub fat_type: FatType,
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub reserved_sectors: u32,
    pub num_fats: u32,
    pub root_entry_count: u32,
    pub sectors_per_fat: u32,
    pub root_cluster: u32,
    pub fat_start_lba: u64,
    pub root_dir_lba: u64,
    pub root_dir_sectors: u32,
    pub data_start_lba: u64,
    pub volume_label: [u8; 11],
}

#[derive(Clone)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
    pub first_cluster: u32,
}

#[inline]
fn read_le16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
fn read_le32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[inline]
fn read_le64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
    ])
}

fn is_valid_bpb(sector: &[u8; 512]) -> bool {
    if sector[510] != 0x55 || sector[511] != 0xAA {
        return false;
    }
    let bps = read_le16(sector, 11);
    let spc = sector[13];
    let num_fats = sector[16];
    if bps != 512 || spc == 0 || !spc.is_power_of_two() || num_fats == 0 || num_fats > 4 {
        return false;
    }
    // Check jump instruction (0xEB xx 0x90 or 0xE9 xx xx)
    sector[0] == 0xEB || sector[0] == 0xE9
}

/// Mount a FAT12/16/32 volume from `blk` (auto-detecting superfloppy, MBR, or GPT partition).
pub fn mount(blk: &VirtIoBlkDevice) -> Result<FatVolume, &'static str> {
    let mut sec0 = [0u8; 512];
    virtio::read_sectors(blk, 0, 1, &mut sec0)?;

    let mut part_lba = 0u64;

    if !is_valid_bpb(&sec0) {
        // Check MBR partition table at +446
        if sec0[510] != 0x55 || sec0[511] != 0xAA {
            return Err("Disk does not contain a valid MBR/GPT signature (0x55AA)");
        }
        let mut found_lba = None;
        for i in 0..4 {
            let off = 446 + i * 16;
            let ptype = sec0[off + 4];
            let lba_start = read_le32(&sec0, off + 8) as u64;
            let num_sec = read_le32(&sec0, off + 12);
            if ptype == 0xEE {
                // GPT protective MBR: inspect GPT header at LBA 1
                let mut gpt_hdr = [0u8; 512];
                virtio::read_sectors(blk, 1, 1, &mut gpt_hdr)?;
                if &gpt_hdr[0..8] == b"EFI PART" {
                    let entries_lba = read_le64(&gpt_hdr, 72);
                    let mut ent_sec = [0u8; 512];
                    virtio::read_sectors(blk, entries_lba, 1, &mut ent_sec)?;
                    for e in 0..4 {
                        let eoff = e * 128;
                        let first_lba = read_le64(&ent_sec, eoff + 32);
                        if first_lba != 0 {
                            found_lba = Some(first_lba);
                            break;
                        }
                    }
                }
                break;
            } else if ptype != 0 && lba_start > 0 && num_sec > 0 {
                found_lba = Some(lba_start);
                break;
            }
        }
        part_lba = found_lba.ok_or("No valid FAT partition found on disk")?;
        virtio::read_sectors(blk, part_lba, 1, &mut sec0)?;
        if !is_valid_bpb(&sec0) {
            return Err("Partition boot sector is not a valid FAT BPB");
        }
    }

    let bytes_per_sector = read_le16(&sec0, 11) as u32;
    let sectors_per_cluster = sec0[13] as u32;
    let reserved_sectors = read_le16(&sec0, 14) as u32;
    let num_fats = sec0[16] as u32;
    let root_entry_count = read_le16(&sec0, 17) as u32;
    let total_sectors_16 = read_le16(&sec0, 19) as u32;
    let total_sectors_32 = read_le32(&sec0, 32);
    let total_sectors = if total_sectors_16 != 0 {
        total_sectors_16
    } else {
        total_sectors_32
    };

    let fat_sz_16 = read_le16(&sec0, 22) as u32;
    let sectors_per_fat = if fat_sz_16 != 0 {
        fat_sz_16
    } else {
        read_le32(&sec0, 36)
    };

    let root_dir_sectors = ((root_entry_count * 32) + (bytes_per_sector - 1)) / bytes_per_sector;
    let fat_start_lba = part_lba + (reserved_sectors as u64);
    let root_dir_lba = fat_start_lba + ((num_fats * sectors_per_fat) as u64);
    let data_start_lba = root_dir_lba + (root_dir_sectors as u64);

    let data_sectors =
        total_sectors.saturating_sub(reserved_sectors + num_fats * sectors_per_fat + root_dir_sectors);
    let count_of_clusters = data_sectors / sectors_per_cluster;

    let fat_type = if root_entry_count == 0 || count_of_clusters >= 65525 {
        FatType::Fat32
    } else if count_of_clusters < 4085 {
        FatType::Fat12
    } else {
        FatType::Fat16
    };

    let root_cluster = if fat_type == FatType::Fat32 {
        read_le32(&sec0, 44)
    } else {
        0
    };

    let mut volume_label = [b' '; 11];
    let label_off = if fat_type == FatType::Fat32 { 71 } else { 43 };
    volume_label.copy_from_slice(&sec0[label_off..label_off + 11]);

    Ok(FatVolume {
        part_lba,
        fat_type,
        bytes_per_sector,
        sectors_per_cluster,
        reserved_sectors,
        num_fats,
        root_entry_count,
        sectors_per_fat,
        root_cluster,
        fat_start_lba,
        root_dir_lba,
        root_dir_sectors,
        data_start_lba,
        volume_label,
    })
}

impl FatVolume {
    pub fn label_str(&self) -> &str {
        let s = core::str::from_utf8(&self.volume_label).unwrap_or("NO NAME");
        s.trim_end()
    }

    fn cluster_to_lba(&self, cluster: u32) -> u64 {
        self.data_start_lba + ((cluster.saturating_sub(2) as u64) * (self.sectors_per_cluster as u64))
    }

    fn next_cluster(&self, blk: &VirtIoBlkDevice, cluster: u32) -> Result<Option<u32>, &'static str> {
        match self.fat_type {
            FatType::Fat12 => {
                let fat_offset = (cluster + (cluster / 2)) as usize;
                let sec_lba = self.fat_start_lba + ((fat_offset / 512) as u64);
                let ent_offset = fat_offset % 512;
                let mut buf = [0u8; 1024];
                virtio::read_sectors(blk, sec_lba, 2, &mut buf)?;
                let raw = read_le16(&buf, ent_offset);
                let val = if (cluster & 1) != 0 {
                    raw >> 4
                } else {
                    raw & 0x0fff
                } as u32;
                if val >= 0x0ff8 || val < 2 {
                    Ok(None)
                } else {
                    Ok(Some(val))
                }
            }
            FatType::Fat16 => {
                let fat_offset = (cluster * 2) as usize;
                let sec_lba = self.fat_start_lba + ((fat_offset / 512) as u64);
                let ent_offset = fat_offset % 512;
                let mut buf = [0u8; 512];
                virtio::read_sectors(blk, sec_lba, 1, &mut buf)?;
                let val = read_le16(&buf, ent_offset) as u32;
                if val >= 0xfff8 || val < 2 {
                    Ok(None)
                } else {
                    Ok(Some(val))
                }
            }
            FatType::Fat32 => {
                let fat_offset = (cluster * 4) as usize;
                let sec_lba = self.fat_start_lba + ((fat_offset / 512) as u64);
                let ent_offset = fat_offset % 512;
                let mut buf = [0u8; 512];
                virtio::read_sectors(blk, sec_lba, 1, &mut buf)?;
                let val = read_le32(&buf, ent_offset) & 0x0fff_ffff;
                if val >= 0x0fff_fff8 || val < 2 {
                    Ok(None)
                } else {
                    Ok(Some(val))
                }
            }
        }
    }

    fn read_cluster_chain(
        &self,
        blk: &VirtIoBlkDevice,
        start_cluster: u32,
        max_bytes: Option<usize>,
    ) -> Result<Vec<u8>, &'static str> {
        let mut out = Vec::new();
        if start_cluster < 2 {
            return Ok(out);
        }
        let cluster_bytes = (self.sectors_per_cluster as usize) * 512;
        let mut cluster_buf = vec![0u8; cluster_bytes];
        let mut cur = Some(start_cluster);
        let mut guard = 0usize;

        while let Some(c) = cur {
            guard += 1;
            if guard > 65536 {
                return Err("FAT cluster chain loop detected");
            }
            let lba = self.cluster_to_lba(c);
            virtio::read_sectors(blk, lba, self.sectors_per_cluster as usize, &mut cluster_buf)?;

            if let Some(limit) = max_bytes {
                let remaining = limit.saturating_sub(out.len());
                let take = remaining.min(cluster_bytes);
                out.extend_from_slice(&cluster_buf[..take]);
                if out.len() >= limit {
                    break;
                }
            } else {
                out.extend_from_slice(&cluster_buf);
            }

            cur = self.next_cluster(blk, c)?;
        }

        Ok(out)
    }

    fn read_dir_bytes(
        &self,
        blk: &VirtIoBlkDevice,
        dir_cluster: Option<u32>,
    ) -> Result<Vec<u8>, &'static str> {
        match dir_cluster {
            None => {
                if self.fat_type == FatType::Fat32 {
                    self.read_cluster_chain(blk, self.root_cluster, None)
                } else {
                    let mut buf = vec![0u8; (self.root_dir_sectors as usize) * 512];
                    virtio::read_sectors(blk, self.root_dir_lba, self.root_dir_sectors as usize, &mut buf)?;
                    Ok(buf)
                }
            }
            Some(c) => self.read_cluster_chain(blk, c, None),
        }
    }

    fn parse_dir_entries(&self, raw: &[u8]) -> Vec<DirEntryInfo> {
        let mut entries = Vec::new();
        let mut lfn_parts: [Option<[u16; 13]>; 20] = [None; 20];
        let mut has_lfn = false;

        for chunk in raw.chunks_exact(32) {
            let first = chunk[0];
            if first == 0x00 {
                break;
            }
            if first == 0xE5 {
                has_lfn = false;
                lfn_parts = [None; 20];
                continue;
            }
            let attr = chunk[11];
            if attr == 0x0F {
                // VFAT Long File Name entry
                let seq = (first & 0x1F) as usize;
                if seq >= 1 && seq <= 20 {
                    let mut chars = [0u16; 13];
                    let offsets = [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                    for (i, &off) in offsets.iter().enumerate() {
                        chars[i] = read_le16(chunk, off);
                    }
                    lfn_parts[seq - 1] = Some(chars);
                    has_lfn = true;
                }
                continue;
            }
            // Skip volume label entries
            if (attr & 0x08) != 0 {
                has_lfn = false;
                lfn_parts = [None; 20];
                continue;
            }

            let name = if has_lfn {
                let mut s = String::new();
                'outer: for part in lfn_parts.iter().flatten() {
                    for &ch in part {
                        if ch == 0x0000 || ch == 0xFFFF {
                            break 'outer;
                        }
                        if let Some(c) = char::from_u32(ch as u32) {
                            s.push(c);
                        }
                    }
                }
                has_lfn = false;
                lfn_parts = [None; 20];
                if s.is_empty() {
                    format_short_name(&chunk[0..11])
                } else {
                    s
                }
            } else {
                format_short_name(&chunk[0..11])
            };

            if name == "." || name == ".." {
                continue;
            }

            let is_dir = (attr & 0x10) != 0;
            let cluster_hi = if self.fat_type == FatType::Fat32 {
                read_le16(chunk, 20) as u32
            } else {
                0
            };
            let cluster_lo = read_le16(chunk, 26) as u32;
            let first_cluster = (cluster_hi << 16) | cluster_lo;
            let size = read_le32(chunk, 28);

            entries.push(DirEntryInfo {
                name,
                is_dir,
                size,
                first_cluster,
            });
        }

        entries
    }

    /// List entries in a directory path (e.g. `"/"` or `"/EFI/BOOT"`).
    pub fn list_dir(
        &self,
        blk: &VirtIoBlkDevice,
        path: &str,
    ) -> Result<Vec<DirEntryInfo>, &'static str> {
        let mut cur_cluster: Option<u32> = None;
        for comp in path.split(|c| c == '/' || c == '\\') {
            if comp.is_empty() || comp == "." {
                continue;
            }
            let dir_bytes = self.read_dir_bytes(blk, cur_cluster)?;
            let list = self.parse_dir_entries(&dir_bytes);
            let found = list
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(comp))
                .ok_or("Directory component not found")?;
            if !found.is_dir {
                return Err("Path component is a file, not a directory");
            }
            cur_cluster = Some(found.first_cluster);
        }
        let dir_bytes = self.read_dir_bytes(blk, cur_cluster)?;
        Ok(self.parse_dir_entries(&dir_bytes))
    }

    /// Read a file by path (e.g. `"/EFI/BOOT/BOOTAA64.EFI"`) into a `Vec<u8>`.
    pub fn read_file(&self, blk: &VirtIoBlkDevice, path: &str) -> Result<Vec<u8>, &'static str> {
        let mut comps: Vec<&str> = path
            .split(|c| c == '/' || c == '\\')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        if comps.is_empty() {
            return Err("Empty file path");
        }
        let file_name = comps.pop().unwrap();
        let mut cur_cluster: Option<u32> = None;

        for dir_comp in comps {
            let dir_bytes = self.read_dir_bytes(blk, cur_cluster)?;
            let list = self.parse_dir_entries(&dir_bytes);
            let found = list
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(dir_comp))
                .ok_or("Parent directory not found")?;
            if !found.is_dir {
                return Err("Parent path component is not a directory");
            }
            cur_cluster = Some(found.first_cluster);
        }

        let dir_bytes = self.read_dir_bytes(blk, cur_cluster)?;
        let list = self.parse_dir_entries(&dir_bytes);
        let file_entry = list
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(file_name))
            .ok_or("File not found")?;

        if file_entry.is_dir {
            return Err("Requested path is a directory, not a file");
        }
        if file_entry.size == 0 {
            return Ok(Vec::new());
        }

        self.read_cluster_chain(blk, file_entry.first_cluster, Some(file_entry.size as usize))
    }
}

fn format_short_name(raw11: &[u8]) -> String {
    let mut s = String::new();
    let base = &raw11[0..8];
    let ext = &raw11[8..11];
    for &b in base {
        if b == b' ' {
            break;
        }
        let ch = if b == 0x05 { 0xE5 } else { b };
        s.push(ch as char);
    }
    if ext[0] != b' ' {
        s.push('.');
        for &b in ext {
            if b == b' ' {
                break;
            }
            s.push(b as char);
        }
    }
    s
}
