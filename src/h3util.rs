use h3_rs::{Error as H3Error, GeoCoord, H3Index};
use regex::Regex;

// H3 indices used as scores must have max resolution
pub const MIN_RESOLUTION: i32 = 0;
pub const MAX_RESOLUTION: i32 = 15;

const H3_RES_OFFSET: u64 = 52;
const H3_RES_MASK: u64 = 15 << H3_RES_OFFSET;
const H3_RES_MASK_NEGATIVE: u64 = !H3_RES_MASK;

// H3 cell index representation
// 1) 1 bit reserved and set to 0,
// 2) 4 bits to indicate the index mode,
// 3) 3 bits reserved and set to 0,
// 4) 4 bits to indicate the cell resolution 0-15,
// 5) 7 bits to indicate the base cell 0-121, and
// 6) 3 bits to indicate each subsequent digit 0-6 from resolution 1 up to the resolution
//    of the cell (45 bits total are reserved for resolutions 1-15)

// |1|   2|  3|   4|      5|6...
//  0 0001 000 0000 0000000 1_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111

// H3 base cells
// 0: dec: 576495936675512319
//    hex: 8001fffffffffff
//    bin: 0b0000_1000_0000_0000_0001_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111

// masks out the top 12 bits
// dec: 4503599627370495
// hex: 0x000FFFFFFFFFFFFF
// bin: 0b0000_0000_0000_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111
const LOW52_MASK: u64 = 0x000FFFFFFFFFFFFF;

// assumes mode == 1 and resolution == 15
// >>52 to get resolution
// dec: 644014746713980928
// hex: 0x08F0000000000000
// bin: 0b0000_1000_1111_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000
const HIGH12_BITS: u64 = 0x08F0000000000000;


// A discussion of why double is used instead of long long for zset scores:
// https://github.com/antirez/redis/issues/6209

// NOTE: we only need the bottom 52 bits of the 64-bit long long if we assume:
//       1) reserved bit == 0      1 bit
//       2) index mode == 1        4 bits
//       3) reserved bits == 0     3 bits
//       4) cell resolution == 15  4 bits
// see:
//       https://h3geo.org/docs/core-library/h3indexing

// convert H3 long long to zset score double
pub fn h3ll_to_score(mut h3ll: u64) -> f64 {
    h3ll &= LOW52_MASK; // Unset top 12 bits
    h3ll as f64
}

// convert zset score double to H3 long long
pub fn score_to_h3ll(score: f64) -> u64 {
    // dec: 644014746713980928
    // hex: 0x08F0000000000000
    // bin: 0b0000_1000_1111_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000
    let mut h3ll = score as u64;
    h3ll |= HIGH12_BITS; // Set high bits
    h3ll
}

// convert string to H3Index, string can be either a valid hex key or long long value
pub fn str_to_h3(h3str: &String) -> Result<H3Index, H3Error> {
    let h3_key_regex: Regex = Regex::new("^(0x)?[0-9A-Za-z]{15}$").unwrap();
    if h3_key_regex.is_match(&h3str) {
        match H3Index::from_str(&h3str) {
            Ok(h3idx) => {
                // println!("h3idx (from key): {:?}", h3idx);
                Ok(h3idx)
            },
            Err(_err) => return Err(H3Error::InvalidString { value: h3str.clone() })
        }
    } else {
        match u64::from_str_radix(h3str.as_str(), 10) {
            Ok(h3ll) => match H3Index::new(h3ll) {
                Ok(h3idx) => {
                    // println!("h3idx (from u64): {:?}", h3idx);
                    Ok(h3idx)
                },
                Err(_err) => return Err(H3Error::InvalidIndex { value: h3ll })
            },
            Err(_err) => return Err(H3Error::FailedConversion)
        }
    }
}

// this bypasses having to convert to and from H3Index
fn get_resolution(h3ll: u64) -> u8 {
    ((h3ll & H3_RES_MASK) >> H3_RES_OFFSET) as u8
}

fn set_resolution(h3ll: u64, new_res: u8) -> u64 {
    (h3ll & H3_RES_MASK_NEGATIVE) | (new_res as u64) << H3_RES_OFFSET
}

/// for a given H3Index as u64, get the lowest valued res 15 child cell
pub fn index_min_child(h3ll: u64) -> u64 {
    let res = get_resolution(h3ll);
    // if res == 15 it's already the max (has no children)
    if res == 15 {
        return h3ll;
    }

    let mut min_child: u64 = set_resolution(h3ll, 15);

    // shift down and back up to zero-out the child index bits
    let child_index_bit_length = (15 - res) * 3;
    min_child = min_child >> (child_index_bit_length as u64);
    min_child = min_child << (child_index_bit_length as u64);

    min_child
}

/// for a given H3Index as u64, get the highest valued res 15 child cell
pub fn index_max_child(h3ll: u64) -> u64 {
    let res = get_resolution(h3ll);
    // if res == 15 it's already the max (has no children)
    if res == 15 {
        return h3ll;
    }

    let mut max_child: u64 = set_resolution(h3ll, 15);

    // shift down and back up to zero-out the child index bits
    let child_index_bit_length = (15 - res) * 3;
    max_child = max_child >> (child_index_bit_length as u64);
    max_child = max_child << (child_index_bit_length as u64);

    // set all child cell indices to highest (6)
    let mut child_index_bits: u64 = 0;
    let mut next_res = res + 1;
    while next_res <= 15 {
        let diff = 15 - next_res;
        child_index_bits += 6 << (3 * diff) as u64;
        next_res += 1;
    }

    max_child |= child_index_bits;
    max_child
}

