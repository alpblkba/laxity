use std::collections::BTreeMap;

const MAGIC: [u8; 2] = *b"LX";
const FRAME_OVERHEAD: usize = 8;
const PLACEMENT_SIZE: usize = 20;
const RECORD_SIZE: usize = 32;
const FLAG_CYCCNT_WRAP: u8 = 1 << 0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub frames: u64,
    pub accepted_frames: u64,
    pub header_frames: u64,
    pub batch_frames: u64,
    pub records: u64,
    pub dropped: u32,
    pub gaps: u64,
    pub false_sync: u64,
    pub crc_rejections: u64,
    pub skipped_bytes: u64,
    pub records_before_header: u64,
    pub wrapped: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub version: u8,
    pub clock_hz: u32,
    pub cyccnt_hz: u32,
    pub seq_next: u32,
    pub stall_available: bool,
    pub stall_populated: bool,
    pub n_regions: u8,
    pub n_models: u8,
    pub record_size: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub id: u8,
    pub name: String,
    pub rel_cost: u16,
    pub arena_addr: u32,
    pub arena_size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub seq: u32,
    pub release_cyc: u32,
    pub exec_cyc: u32,
    pub cpu_cyc: u32,
    pub stall_cyc: u32,
    pub model_id: u16,
    pub region_id: u8,
    pub flags: u8,
    pub aggressor_idx: u16,
    pub padding: u16,
    pub reserved: u32,
}

#[derive(Debug, Default)]
pub struct ParseDelta {
    pub ascii: Vec<u8>,
    pub valid_frames: u64,
    pub rejected_candidates: u64,
    pub records: u64,
}

#[derive(Debug, Default)]
pub struct Parser {
    pending: Vec<u8>,
    seen_header: bool,
    expect_seq: Option<u64>,
    pub stats: Stats,
    pub metadata: Option<Metadata>,
    pub placements: BTreeMap<u8, Placement>,
    pub region_exec: BTreeMap<u8, Vec<u32>>,
    pub latest_record: Option<Record>,
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> ParseDelta {
        self.pending.extend_from_slice(bytes);
        self.parse(false)
    }

    pub fn finish(&mut self) -> ParseDelta {
        self.parse(true)
    }

    pub fn median_exec(&self, region_id: u8) -> Option<u32> {
        let values = self.region_exec.get(&region_id)?;
        values.get(values.len() / 2).copied()
    }

    fn parse(&mut self, eof: bool) -> ParseDelta {
        let mut delta = ParseDelta::default();
        let mut pos = 0;

        while pos < self.pending.len() {
            let remaining = self.pending.len() - pos;
            if remaining < FRAME_OVERHEAD {
                if eof {
                    self.skip_tail(pos, &mut delta);
                    pos = self.pending.len();
                    break;
                }

                let could_be_frame = self.pending[pos] == MAGIC[0]
                    && (remaining == 1 || self.pending[pos + 1] == MAGIC[1]);
                if could_be_frame {
                    break;
                }

                self.skip_byte(pos, &mut delta);
                pos += 1;
                continue;
            }

            if self.pending[pos..pos + 2] != MAGIC {
                self.skip_byte(pos, &mut delta);
                pos += 1;
                continue;
            }

            let version = self.pending[pos + 2];
            let frame_type = self.pending[pos + 3];
            let payload_len = u16_at(&self.pending, pos + 4) as usize;
            let wanted_crc = u16_at(&self.pending, pos + 6);
            let body = pos + FRAME_OVERHEAD;
            let frame_end = body + payload_len;

            let known_header = matches!(version, 1 | 2) && matches!(frame_type, 0 | 1);
            if known_header && frame_end > self.pending.len() && !eof {
                break;
            }

            let crc_checked = frame_end <= self.pending.len();
            let crc_ok = crc_checked && crc16(&self.pending[body..frame_end]) == wanted_crc;
            if !known_header || !crc_ok {
                if known_header && crc_checked {
                    self.stats.crc_rejections += 1;
                }
                self.reject_candidate(pos, &mut delta);
                pos += 1;
                continue;
            }

            self.stats.frames += 1;
            let payload = self.pending[body..frame_end].to_vec();
            let accepted = match frame_type {
                0 => self.parse_header(version, &payload),
                1 => self.parse_batch(&payload, &mut delta),
                _ => unreachable!(),
            };

            if accepted {
                self.stats.accepted_frames += 1;
                delta.valid_frames += 1;
                pos = frame_end;
            } else {
                self.reject_candidate(pos, &mut delta);
                pos += 1;
            }
        }

        if pos > 0 {
            self.pending.drain(..pos);
        }
        delta
    }

    fn parse_header(&mut self, version: u8, payload: &[u8]) -> bool {
        let fixed = if version == 1 { 20 } else { 40 };
        let n_regions = payload.get(17).copied().unwrap_or(0);
        if payload.len() != fixed + n_regions as usize * PLACEMENT_SIZE {
            return false;
        }

        self.metadata = Some(Metadata {
            version,
            clock_hz: u32_at(payload, 0),
            cyccnt_hz: u32_at(payload, 4),
            seq_next: u32_at(payload, 8),
            stall_available: payload[16] & 1 != 0,
            stall_populated: payload[16] & 2 != 0,
            n_regions,
            n_models: payload[18],
            record_size: payload[19],
        });
        self.stats.dropped = self.stats.dropped.max(u32_at(payload, 12));
        self.stats.header_frames += 1;
        self.seen_header = true;
        self.placements.clear();

        for index in 0..n_regions as usize {
            let offset = fixed + index * PLACEMENT_SIZE;
            let raw_name = &payload[offset + 12..offset + 20];
            let name_end = raw_name.iter().position(|byte| *byte == 0).unwrap_or(8);
            let placement = Placement {
                id: payload[offset],
                name: String::from_utf8_lossy(&raw_name[..name_end]).into_owned(),
                rel_cost: u16_at(payload, offset + 2),
                arena_addr: u32_at(payload, offset + 4),
                arena_size: u32_at(payload, offset + 8),
            };
            self.placements.insert(placement.id, placement);
        }

        true
    }

    fn parse_batch(&mut self, payload: &[u8], delta: &mut ParseDelta) -> bool {
        if !payload.len().is_multiple_of(RECORD_SIZE) {
            return false;
        }

        let count = payload.len() / RECORD_SIZE;
        self.stats.batch_frames += 1;
        if !self.seen_header {
            self.stats.records_before_header += count as u64;
            return true;
        }

        for chunk in payload.chunks_exact(RECORD_SIZE) {
            let record = Record {
                seq: u32_at(chunk, 0),
                release_cyc: u32_at(chunk, 4),
                exec_cyc: u32_at(chunk, 8),
                cpu_cyc: u32_at(chunk, 12),
                stall_cyc: u32_at(chunk, 16),
                model_id: u16_at(chunk, 20),
                region_id: chunk[22],
                flags: chunk[23],
                aggressor_idx: u16_at(chunk, 24),
                padding: u16_at(chunk, 26),
                reserved: u32_at(chunk, 28),
            };

            if self
                .expect_seq
                .is_some_and(|expected| record.seq as u64 != expected)
            {
                self.stats.gaps += 1;
            }
            self.expect_seq = Some(record.seq as u64 + 1);
            if record.flags & FLAG_CYCCNT_WRAP != 0 {
                self.stats.wrapped += 1;
            }

            let values = self.region_exec.entry(record.region_id).or_default();
            let index = values.partition_point(|value| *value <= record.exec_cyc);
            values.insert(index, record.exec_cyc);
            self.latest_record = Some(record);
        }

        self.stats.records += count as u64;
        delta.records += count as u64;
        true
    }

    fn skip_byte(&mut self, pos: usize, delta: &mut ParseDelta) {
        delta.ascii.push(self.pending[pos]);
        self.stats.skipped_bytes += 1;
    }

    fn skip_tail(&mut self, pos: usize, delta: &mut ParseDelta) {
        delta.ascii.extend_from_slice(&self.pending[pos..]);
        self.stats.skipped_bytes += (self.pending.len() - pos) as u64;
    }

    fn reject_candidate(&mut self, pos: usize, delta: &mut ParseDelta) {
        self.skip_byte(pos, delta);
        self.stats.false_sync += 1;
        delta.rejected_candidates += 1;
    }
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for byte in data {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(frame_type: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![b'L', b'X', 2, frame_type];
        out.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&crc16(payload).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn header() -> Vec<u8> {
        let mut payload = vec![0; 60];
        payload[0..4].copy_from_slice(&160_000_000u32.to_le_bytes());
        payload[4..8].copy_from_slice(&159_999_900u32.to_le_bytes());
        payload[16] = 1;
        payload[17] = 1;
        payload[19] = 32;
        payload[40] = 3;
        payload[42..44].copy_from_slice(&1000u16.to_le_bytes());
        payload[44..48].copy_from_slice(&0x2004_0000u32.to_le_bytes());
        payload[48..52].copy_from_slice(&303_104u32.to_le_bytes());
        payload[52..57].copy_from_slice(b"SRAM3");
        frame(0, &payload)
    }

    fn batch(seq: u32, exec: u32) -> Vec<u8> {
        let mut payload = vec![0; RECORD_SIZE];
        payload[0..4].copy_from_slice(&seq.to_le_bytes());
        payload[8..12].copy_from_slice(&exec.to_le_bytes());
        payload[22] = 3;
        frame(1, &payload)
    }

    #[test]
    fn crc_matches_ccitt_false_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
    }

    #[test]
    fn split_reads_match_one_complete_read() {
        let mut stream = b"boot ok\r\nLX not a frame\r\n".to_vec();
        stream.extend(header());
        stream.extend(batch(7, 320_901));
        stream.extend(batch(9, 352_088));

        let mut whole = Parser::default();
        let mut whole_ascii = whole.feed(&stream).ascii;
        whole_ascii.extend(whole.finish().ascii);

        for split in 0..=stream.len() {
            let mut split_parser = Parser::default();
            let mut split_ascii = split_parser.feed(&stream[..split]).ascii;
            split_ascii.extend(split_parser.feed(&stream[split..]).ascii);
            split_ascii.extend(split_parser.finish().ascii);

            assert_eq!(split_parser.stats, whole.stats, "split at {split}");
            assert_eq!(split_parser.metadata, whole.metadata, "split at {split}");
            assert_eq!(
                split_parser.placements, whole.placements,
                "split at {split}"
            );
            assert_eq!(
                split_parser.region_exec, whole.region_exec,
                "split at {split}"
            );
            assert_eq!(split_ascii, whole_ascii, "split at {split}");
        }

        assert_eq!(whole.stats.frames, 3);
        assert_eq!(whole.stats.records, 2);
        assert_eq!(whole.stats.gaps, 1);
        assert_eq!(whole.stats.false_sync, 1);
        assert_eq!(whole.median_exec(3), Some(352_088));
    }

    #[test]
    fn a_nested_candidate_does_not_make_results_depend_on_read_boundaries() {
        let nested = header();
        let mut payload = vec![0; RECORD_SIZE * 3];
        payload[..nested.len()].copy_from_slice(&nested);
        let outer = frame(1, &payload);
        let split = FRAME_OVERHEAD + nested.len();

        let mut whole = Parser::default();
        whole.feed(&outer);

        let mut chunked = Parser::default();
        assert_eq!(chunked.feed(&outer[..split]).valid_frames, 0);
        chunked.feed(&outer[split..]);

        assert_eq!(chunked.stats, whole.stats);
        assert_eq!(chunked.metadata, whole.metadata);
        assert_eq!(chunked.region_exec, whole.region_exec);
        assert_eq!(whole.stats.frames, 1);
        assert_eq!(whole.stats.batch_frames, 1);
        assert_eq!(whole.stats.records_before_header, 3);
    }
}
