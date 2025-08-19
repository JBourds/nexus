//! Miscellaneous helper functions.
use std::collections::HashMap;
use std::hash::Hash;

pub fn format_u8_buf(buf: &[u8]) -> String {
    match core::str::from_utf8(buf) {
        Ok(s) => s.to_string(),
        Err(_) => format!("<{} bytes>", buf.len()),
    }
}

/// Flip bits when `flips` evaluates to true.
/// Returns a tuple with the number of times iterated and the number of bits
/// flipped.
pub fn flip_bits(buf: &mut [u8], flips: impl IntoIterator<Item = bool>) -> (usize, usize) {
    let mut flips = flips.into_iter();
    let mut count = 0;
    let mut flipped = 0;
    for byte in buf {
        for index in 0..u8::BITS {
            match flips.next() {
                Some(true) => {
                    count += 1;
                    flipped += 1;
                    *byte ^= 1 << index;
                }
                Some(false) => {
                    count += 1;
                }
                None => return (count, flipped),
            }
        }
    }
    (count, flipped)
}

pub fn make_handles<T>(iter: impl IntoIterator<Item = T>) -> HashMap<T, usize>
where
    T: Hash + Eq,
{
    let mut next_index = 0;
    let mut handles = HashMap::new();
    for item in iter {
        if handles.contains_key(&item) {
            continue;
        }
        handles.insert(item, next_index);
        next_index += 1;
    }
    handles
}

pub fn unzip<T1, T2>(iter: impl IntoIterator<Item = (T1, T2)>) -> (Vec<T1>, Vec<T2>) {
    iter.into_iter().fold(
        (Vec::new(), Vec::new()),
        |(mut vec1, mut vec2), (val1, val2)| {
            vec1.push(val1);
            vec2.push(val2);
            (vec1, vec2)
        },
    )
}
