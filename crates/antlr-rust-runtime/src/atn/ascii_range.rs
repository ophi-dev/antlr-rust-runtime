//! Exact ASCII range descriptors and range-prefix scanners.

const ASCII_SYMBOLS: usize = 128;
pub(super) const MAX_RANGES: usize = 4;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(super) struct AsciiRange {
    pub(super) low: u8,
    pub(super) high: u8,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct AsciiRanges {
    ranges: [AsciiRange; MAX_RANGES],
    count: u8,
}

impl AsciiRanges {
    pub(super) fn from_self_loops(row: &[u16; ASCII_SYMBOLS], state: u16) -> Option<Self> {
        let mut ranges = [AsciiRange::default(); MAX_RANGES];
        let mut count = 0;
        let mut start = None;

        for (symbol, &target) in row.iter().enumerate() {
            if target == state {
                start.get_or_insert(symbol);
                continue;
            }
            if let Some(low) = start.take() {
                if count == MAX_RANGES {
                    return None;
                }
                ranges[count] = AsciiRange {
                    low: u8::try_from(low).expect("ASCII range start overflow"),
                    high: u8::try_from(symbol - 1).expect("ASCII range end overflow"),
                };
                count += 1;
            }
        }
        if let Some(low) = start {
            if count == MAX_RANGES {
                return None;
            }
            ranges[count] = AsciiRange {
                low: u8::try_from(low).expect("ASCII range start overflow"),
                high: 127,
            };
            count += 1;
        }

        Self::new(u8::try_from(count).expect("range count overflow"), ranges)
    }

    pub(super) fn new(count: u8, ranges: [AsciiRange; MAX_RANGES]) -> Option<Self> {
        let count = usize::from(count);
        if !(1..=MAX_RANGES).contains(&count) {
            return None;
        }
        if ranges[..count]
            .iter()
            .any(|range| range.low > range.high || !range.high.is_ascii())
        {
            return None;
        }
        if ranges[..count]
            .windows(2)
            .any(|pair| pair[0].high + 1 >= pair[1].low)
        {
            return None;
        }
        if ranges[count..]
            .iter()
            .any(|range| *range != AsciiRange::default())
        {
            return None;
        }
        Some(Self {
            ranges,
            count: u8::try_from(count).expect("validated range count fits u8"),
        })
    }

    pub(super) const fn count(self) -> u8 {
        self.count
    }

    pub(super) fn as_slice(&self) -> &[AsciiRange] {
        &self.ranges[..usize::from(self.count)]
    }

    pub(super) fn packed_words(self) -> [u32; 2] {
        let mut words = [0_u32; 2];
        for (index, range) in self.as_slice().iter().enumerate() {
            let shift = (index % 2) * 16;
            words[index / 2] |= u32::from(range.low) << shift;
            words[index / 2] |= u32::from(range.high) << (shift + 8);
        }
        words
    }

    pub(super) fn from_packed(count: u8, words: [u32; 2]) -> Option<Self> {
        let mut ranges = [AsciiRange::default(); MAX_RANGES];
        for (index, range) in ranges.iter_mut().enumerate() {
            let shift = (index % 2) * 16;
            range.low = (words[index / 2] >> shift) as u8;
            range.high = (words[index / 2] >> (shift + 8)) as u8;
        }
        Self::new(count, ranges)
    }

    #[cfg(test)]
    fn contains(self, byte: u8) -> bool {
        self.as_slice()
            .iter()
            .any(|range| byte >= range.low && byte <= range.high)
    }

    #[cfg(any(feature = "perf-counters", test))]
    pub(super) fn class(self) -> AsciiRangeClass {
        let mut any_digit = false;
        let mut any_alpha = false;
        let mut all_whitespace = true;
        let mut all_number = true;
        let mut all_identifier = true;
        for range in self.as_slice() {
            for byte in range.low..=range.high {
                any_digit |= byte.is_ascii_digit();
                any_alpha |= byte.is_ascii_alphabetic();
                all_whitespace &= byte.is_ascii_whitespace();
                all_number &= byte.is_ascii_hexdigit() || matches!(byte, b'_' | b'\'');
                all_identifier &= byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$');
            }
        }
        if all_whitespace {
            return AsciiRangeClass::Whitespace;
        }
        if any_digit && all_number {
            return AsciiRangeClass::Number;
        }
        if any_alpha && all_identifier {
            return AsciiRangeClass::Identifier;
        }
        AsciiRangeClass::Other
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(any(feature = "perf-counters", test))]
pub(crate) enum AsciiRangeClass {
    Identifier,
    Number,
    Whitespace,
    Other,
}

#[inline]
pub(super) fn scan_scalar(ranges: AsciiRanges, input: &[u8]) -> usize {
    let active_ranges = ranges.as_slice();
    input
        .iter()
        .position(|&byte| {
            !active_ranges
                .iter()
                .any(|range| byte >= range.low && byte <= range.high)
        })
        .unwrap_or(input.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(items: &[(u8, u8)]) -> AsciiRanges {
        let mut ranges = [AsciiRange::default(); MAX_RANGES];
        for (slot, &(low, high)) in ranges.iter_mut().zip(items) {
            *slot = AsciiRange { low, high };
        }
        AsciiRanges::new(items.len() as u8, ranges).expect("test ranges should be canonical")
    }

    #[test]
    fn descriptors_are_canonical_and_round_trip_packed_words() {
        let descriptor = ranges(&[(b'0', b'9'), (b'A', b'Z'), (b'_', b'_'), (b'a', b'z')]);
        assert_eq!(
            AsciiRanges::from_packed(descriptor.count(), descriptor.packed_words()),
            Some(descriptor)
        );

        let mut adjacent = [AsciiRange::default(); MAX_RANGES];
        adjacent[0] = AsciiRange {
            low: b'a',
            high: b'm',
        };
        adjacent[1] = AsciiRange {
            low: b'n',
            high: b'z',
        };
        assert_eq!(AsciiRanges::new(2, adjacent), None);

        let mut non_ascii = [AsciiRange::default(); MAX_RANGES];
        non_ascii[0] = AsciiRange {
            low: 127,
            high: 128,
        };
        assert_eq!(AsciiRanges::new(1, non_ascii), None);
    }

    #[test]
    fn randomized_range_scans_match_dfa_rows() {
        let mut random = 0x80A5_5EED_u32;
        for count in 1..=MAX_RANGES {
            for _ in 0..64 {
                let mut row = [0_u16; ASCII_SYMBOLS];
                let state = 7;
                let mut cursor = (next_random(&mut random) & 7) as usize;
                for _ in 0..count {
                    let width = (next_random(&mut random) % 12) as usize;
                    let high = cursor + width;
                    row[cursor..=high].fill(state);
                    cursor = high + 2 + (next_random(&mut random) % 8) as usize;
                }
                let descriptor = AsciiRanges::from_self_loops(&row, state)
                    .expect("generated row should fit the descriptor bound");
                assert_eq!(usize::from(descriptor.count()), count);
                for byte in 0_u8..=127 {
                    assert_eq!(descriptor.contains(byte), row[usize::from(byte)] == state);
                }

                let accepted: Vec<u8> = (0_u8..=127)
                    .filter(|&byte| descriptor.contains(byte))
                    .collect();
                let rejected: Vec<u8> = (0_u8..=u8::MAX)
                    .filter(|&byte| !descriptor.contains(byte))
                    .collect();
                for len in [0, 1, 7, 15, 31, 32, 47, 63, 64, 95, 127, 255] {
                    let mut input = Vec::with_capacity(len + 1);
                    for _ in 0..len {
                        let index = next_random(&mut random) as usize % accepted.len();
                        input.push(accepted[index]);
                    }
                    if next_random(&mut random) & 1 != 0 {
                        let index = next_random(&mut random) as usize % rejected.len();
                        input.push(rejected[index]);
                    }
                    assert_scan_matches_row(descriptor, &row, state, &input);
                }

                for len in [1, 17, 65, 260] {
                    let mut input = Vec::with_capacity(len);
                    for _ in 0..len {
                        input.push(next_random(&mut random).to_le_bytes()[0]);
                    }
                    assert_scan_matches_row(descriptor, &row, state, &input);
                }
            }
        }
    }

    fn next_random(random: &mut u32) -> u32 {
        *random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *random
    }

    fn assert_scan_matches_row(
        descriptor: AsciiRanges,
        row: &[u16; ASCII_SYMBOLS],
        state: u16,
        input: &[u8],
    ) {
        let expected = input
            .iter()
            .position(|&byte| !byte.is_ascii() || row[usize::from(byte)] != state)
            .unwrap_or(input.len());
        assert_eq!(scan_scalar(descriptor, input), expected);
    }

    #[test]
    fn classifies_common_range_shapes_without_grammar_names() {
        assert_eq!(
            ranges(&[(b'0', b'9'), (b'A', b'Z'), (b'_', b'_'), (b'a', b'z')]).class(),
            AsciiRangeClass::Identifier
        );
        assert_eq!(
            ranges(&[(b'0', b'9'), (b'A', b'F'), (b'a', b'f')]).class(),
            AsciiRangeClass::Number
        );
        assert_eq!(
            ranges(&[(b'\t', b'\n'), (b'\x0c', b'\r'), (b' ', b' ')]).class(),
            AsciiRangeClass::Whitespace
        );
    }
}
