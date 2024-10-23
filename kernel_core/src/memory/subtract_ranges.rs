//! Iterator over remaining regions from subtracting reserved regions from a whole block of memory.
//! See [`subtract_ranges`].

/// An iterator over the ranges in a whole memory range that exclude specified sub-ranges.
///
/// This struct is created by the [`subtract_ranges`] function. It implements the [`Iterator`] trait,
/// yielding ranges that are part of the whole range but not part of any sub-range.
pub struct SubtractRanges<I>
where
    I: Iterator<Item = (*mut u8, usize)>,
{
    whole_start: usize, // Start address of the whole range as usize.
    whole_end: usize,   // End address of the whole range.
    current_pos: usize, // Current position in the whole range.
    ranges: I,          // Iterator over sub-ranges to exclude.
    current_subrange: Option<(usize, usize)>, // Current sub-range being processed (start, end).
}

/// Returns an iterator over the ranges in `whole` that exclude the ranges provided by `ranges`.
///
/// This function performs interval subtraction, effectively subtracting the provided `ranges`
/// from the `whole` range. It takes a single continuous memory range `whole` (specified by a
/// pointer and length) and an iterator over sub-ranges to exclude. The returned iterator
/// yields the ranges of `whole` that are not covered by any of the sub-ranges.
///
/// # Arguments
///
/// * `whole` - A tuple containing the starting pointer and length in bytes of the whole range.
/// * `ranges` - An iterator over sub-ranges to exclude from the whole range. Each sub-range is
///   specified as a tuple of starting pointer and length in bytes. **The sub-ranges must be
///   ordered by starting address and must not overlap.**
///
/// # Returns
///
/// An iterator that yields tuples of starting pointers and lengths in bytes for the ranges
/// that are part of the whole range but not part of any sub-range.
///
/// # Notes
///
/// * The `ranges` iterator must yield sub-ranges in order by starting address and must not
///   contain overlapping sub-ranges. If these conditions are not met, the behavior is undefined.
pub fn subtract_ranges<I>(whole: (*mut u8, usize), ranges: I) -> SubtractRanges<I>
where
    I: Iterator<Item = (*mut u8, usize)>,
{
    let (whole_start_ptr, whole_len) = whole;
    let whole_start = whole_start_ptr as usize;
    let whole_end = whole_start.saturating_add(whole_len);

    SubtractRanges {
        whole_start,
        whole_end,
        current_pos: whole_start,
        ranges,
        current_subrange: None,
    }
}

impl<I> Iterator for SubtractRanges<I>
where
    I: Iterator<Item = (*mut u8, usize)>,
{
    type Item = (*mut u8, usize);

    /// Advances the iterator and returns the next non-excluded range.
    ///
    /// Returns `None` when there are no more non-excluded ranges to return.
    fn next(&mut self) -> Option<Self::Item> {
        while self.current_pos < self.whole_end {
            // Load the next sub-range if needed.
            if self.current_subrange.is_none() {
                // Load the next sub-range.
                self.current_subrange = self.ranges.find_map(|(start_ptr, len)| {
                    let start = start_ptr as usize;
                    let end = start.saturating_add(len);
                    let sub_start = start.max(self.whole_start);
                    let sub_end = end.min(self.whole_end);
                    if sub_start >= sub_end {
                        // Sub-range is outside the whole range or empty, skip it.
                        None
                    } else if sub_end <= self.current_pos {
                        // Sub-range ends before current position, skip it.
                        None
                    } else {
                        // Valid sub-range found.
                        Some((sub_start, sub_end))
                    }
                });
            }

            if let Some((sub_start, sub_end)) = self.current_subrange {
                if self.current_pos < sub_start {
                    // There is a gap between current position and sub_start.
                    let result_start = self.current_pos;
                    let result_end = sub_start;
                    let len = result_end - result_start;
                    self.current_pos = sub_start; // Move current position to sub_start
                                                  // Do not advance subrange, as we have not processed it yet.
                    return Some((result_start as *mut u8, len));
                }

                // current_pos >= sub_start
                self.current_pos = sub_end; // Skip over the sub-range.
                self.current_subrange = None; // Prepare to load next sub-range.
            } else {
                // No more sub-ranges, output the remaining range.
                let result_start = self.current_pos;
                let result_end = self.whole_end;
                let len = result_end - result_start;
                self.current_pos = self.whole_end;
                return (len > 0).then_some((result_start as *mut u8, len));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Helper function to sort and merge overlapping sub-ranges.
    ///
    /// This function is used in tests to prepare sub-ranges before passing them to `subtract_ranges`.
    fn sort_and_merge_subranges(sub_ranges: Vec<(*mut u8, usize)>) -> Vec<(*mut u8, usize)> {
        // Convert to usize for sorting.
        let mut ranges: Vec<(usize, usize)> = sub_ranges
            .iter()
            .map(|(ptr, len)| (*ptr as usize, (*ptr as usize).saturating_add(*len)))
            .collect();

        // Sort ranges by start address.
        ranges.sort_by_key(|&(start, _)| start);

        // Merge overlapping ranges.
        let mut merged_ranges = Vec::new();
        for (start, end) in ranges {
            if let Some((_, last_end)) = merged_ranges.last_mut() {
                if start <= *last_end {
                    // Ranges overlap or touch, merge them.
                    *last_end = (*last_end).max(end);
                } else {
                    // Non-overlapping, add new range.
                    merged_ranges.push((start, end));
                }
            } else {
                merged_ranges.push((start, end));
            }
        }

        // Convert back to pointers and lengths.
        merged_ranges
            .into_iter()
            .map(|(start, end)| (start as *mut u8, end.saturating_sub(start)))
            .collect()
    }

    #[test]
    fn test_no_subranges() {
        let whole = (0x1000 as *mut u8, 1000);
        let sub_ranges = vec![];
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], whole);
    }

    #[test]
    fn test_subranges_within_whole() {
        let whole = (0x1000 as *mut u8, 1000); // 4096 to 5096
        let sub_ranges = vec![
            (0x1100 as *mut u8, 100), // 4352 to 4452
            (0x1200 as *mut u8, 200), // 4608 to 4808
        ];
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();

        // Expected ranges:
        // 1. 4096 to 4352 (256 bytes)
        // 2. 4452 to 4608 (156 bytes)
        // 3. 4808 to 5096 (288 bytes)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], (0x1000 as *mut u8, 256)); // 4096 to 4352
        assert_eq!(result[1], (0x1164 as *mut u8, 156)); // 4452 to 4608
        assert_eq!(result[2], (0x12C8 as *mut u8, 288)); // 4808 to 5096
    }

    #[test]
    fn test_subranges_outside_whole() {
        let whole = (0x1000 as *mut u8, 1000); // 4096 to 5096
        let sub_ranges = vec![
            (0x0F00 as *mut u8, 256), // 3840 to 4096
            (0x13E8 as *mut u8, 500), // 5096 to 5596
            (0x1500 as *mut u8, 100), // 5376 to 5476
        ];
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], whole);
    }

    #[test]
    fn test_overlapping_subranges() {
        let whole = (0x1000 as *mut u8, 1000);
        let sub_ranges = vec![
            (0x1100 as *mut u8, 200), // 4352 to 4552
            (0x1150 as *mut u8, 300), // 4432 to 4732 (overlaps previous)
        ];
        let sub_ranges = sort_and_merge_subranges(sub_ranges);
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();

        // After merging, sub-range is 4352 to 4732
        // Expected ranges:
        // 1. 4096 to 4352 (256 bytes)
        // 2. 4732 to 5096 (364 bytes)
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], (0x1000 as *mut u8, 256)); // 4096 to 4352
        assert_eq!(result[1], (0x127C as *mut u8, 364)); // 4732 to 5096
    }

    #[test]
    fn test_subranges_cover_whole() {
        let whole = (0x1000 as *mut u8, 1000);
        let sub_ranges = vec![(0x0F00 as *mut u8, 2000)]; // 3840 to 5840
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_subranges_with_gaps() {
        let whole = (0x1000 as *mut u8, 1000); // 4096 to 5096
        let sub_ranges = vec![
            (0x1000 as *mut u8, 100), // 4096 to 4196
            (0x1100 as *mut u8, 100), // 4352 to 4452
            (0x1200 as *mut u8, 100), // 4608 to 4708
            (0x1300 as *mut u8, 100), // 4864 to 4964
        ];
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();

        // Expected ranges:
        // 1. 4196 to 4352 (156 bytes)
        // 2. 4452 to 4608 (156 bytes)
        // 3. 4708 to 4864 (156 bytes)
        // 4. 4964 to 5096 (132 bytes)
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], (0x1064 as *mut u8, 156)); // 4196 to 4352
        assert_eq!(result[1], (0x1164 as *mut u8, 156)); // 4452 to 4608
        assert_eq!(result[2], (0x1264 as *mut u8, 156)); // 4708 to 4864
        assert_eq!(result[3], (0x1364 as *mut u8, 132)); // 4964 to 5096
    }

    #[test]
    fn test_subranges_unordered() {
        let whole = (0x1000 as *mut u8, 1000); // 4096 to 5096
        let sub_ranges = vec![
            (0x1200 as *mut u8, 100), // 4608 to 4708
            (0x1100 as *mut u8, 100), // 4352 to 4452
            (0x1300 as *mut u8, 100), // 4864 to 4964
        ];
        let sub_ranges = sort_and_merge_subranges(sub_ranges);
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();

        // Expected ranges:
        // 1. 4096 to 4352 (256 bytes)
        // 2. 4452 to 4608 (156 bytes)
        // 3. 4708 to 4864 (156 bytes)
        // 4. 4964 to 5096 (132 bytes)
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], (0x1000 as *mut u8, 256)); // 4096 to 4352
        assert_eq!(result[1], (0x1164 as *mut u8, 156)); // 4452 to 4608
        assert_eq!(result[2], (0x1264 as *mut u8, 156)); // 4708 to 4864
        assert_eq!(result[3], (0x1364 as *mut u8, 132)); // 4964 to 5096
    }

    #[test]
    fn test_subranges_with_overlaps_and_unordered() {
        let whole = (0x1000 as *mut u8, 1000);
        let sub_ranges = vec![
            (0x1200 as *mut u8, 300), // 4608 to 4908
            (0x1100 as *mut u8, 200), // 4352 to 4552
            (0x1300 as *mut u8, 100), // 4864 to 4964
        ];
        let sub_ranges = sort_and_merge_subranges(sub_ranges);
        let result: Vec<(*mut u8, usize)> =
            subtract_ranges(whole, sub_ranges.into_iter()).collect();

        // After merging, sub-ranges are:
        // 1. 4352 to 4552
        // 2. 4608 to 4964 (merged 4608 to 4908 and 4864 to 4964)
        // Expected ranges:
        // 1. 4096 to 4352 (256 bytes)
        // 2. 4552 to 4608 (56 bytes)
        // 3. 4964 to 5096 (132 bytes)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], (0x1000 as *mut u8, 256)); // 4096 to 4352
        assert_eq!(result[1], (0x11C8 as *mut u8, 56)); // 4552 to 4608
        assert_eq!(result[2], (0x1364 as *mut u8, 132)); // 4964 to 5096
    }

    #[test]
    fn real_world_4gib() {
        let whole = (0xffff_0000_40000000 as *mut u8, 0x100000000);
        let ranges = [
            (0xffff000041000000 as *mut u8, 0x41bc00),
            (0xffff0000ffefc000 as *mut u8, 0x2000),
        ];

        let res: Vec<_> = subtract_ranges(whole, ranges.into_iter()).collect();
        println!("result = {res:x?}");

        assert_eq!(res.len(), 3);
        assert_eq!(res[0], (0xffff000040000000 as *mut u8, 0x0100_0000));
        assert_eq!(res[1], (0xffff00004141bc00 as *mut u8, 0xbeae_0400));
        assert_eq!(res[2], (0xffff0000ffefe000 as *mut u8, 0x4010_2000));
    }
}
