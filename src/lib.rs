#[macro_use]
extern crate redis_module;

use std::os::raw::c_int;
use regex::Regex;

use h3_rs::{GeoCoord, H3Index, Error as H3Error};
use redis_module::{NextArg, raw as rawmod, RedisString};
use redis_module::{Context, RedisError, RedisResult, RedisValue};

// H3 indices used as scores must have this resolution
const MAX_RESOLUTION: i32 = 15;

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
fn h3ll_to_score(mut h3ll: u64) -> f64 {
    h3ll &= LOW52_MASK; // Unset top 12 bits
    h3ll as f64
}

// convert zset score double to H3 long long
fn score_to_h3ll(score: f64) -> u64 {
    // dec: 644014746713980928
    // hex: 0x08F0000000000000
    // bin: 0b0000_1000_1111_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000
    let mut h3ll = score as u64;
    h3ll |= HIGH12_BITS; // Set high bits
    h3ll
}

// convert string to H3Index, string can be either a valid hex key or long long value
fn str_to_h3(h3str: &String) -> Result<H3Index, H3Error> {
    let h3_key_regex: Regex = Regex::new("^(0x)?[0-9A-Za-z]{15}$").unwrap();
    if (h3_key_regex.is_match(&h3str)) {
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
fn index_min_child(h3ll: u64) -> u64 {
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
fn index_max_child(h3ll: u64) -> u64 {
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
        child_index_bits += (6 << (3 * diff) as u64);
        next_res += 1;
    }

    max_child |= child_index_bits;
    max_child
}

///
/// H3.STATUS
///
/// a command to check status of H3 module (e.g. whether it is successfully loaded)
///
fn h3status_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    let status = String::from("Ok");
    Ok(status.into())
}

///
/// H3.ADD key lng lat name [lng2 lat2 name2 ... lngN latN nameN]
///
/// this is an attempted rust "translation" of geoaddCommand into an
/// equivalent command for H3
///
fn h3add_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    if args.len() < 5 || (args.len() - 2) % 3 != 0 {
        return Err(RedisError::from(
            "syntax error. Try H3.ADD key [lng1] [lat1] [name1] [lng2] [lat2] [name2] ... "
        ));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let elements: usize = args.len() / 3;
    let argc: usize = 2+elements*2; /* ZADD key score elem ... */

    let mut newargs: Vec<String> = Vec::with_capacity(argc);
    newargs.push(key);

    /* Create the argument vector to call ZADD in order to add all
     * the score,value pairs to the requested zset, where score is actually
     * an encoded version of lat,long. */
    while args.len() > 0 {
        match (args.next_f64(), args.next_f64()) {
            (Ok(lng), Ok(lat)) => {
                let name = args.next_string()?;
                // TODO: need to validate lng/lat (should probably happen in GeoCoord::new)
                let coord: GeoCoord = GeoCoord::new(lat, lng);
                let h3_from_coord = coord.to_h3(MAX_RESOLUTION).unwrap();
                let h3ll: u64 = u64::from_str_radix(h3_from_coord.to_string().as_str(), 16).unwrap();
                let score: f64 = h3ll_to_score(h3ll);

                newargs.push(format!("{}", score));
                newargs.push(name.clone());
            },
            _ => return Err(RedisError::from("Invalid lng or lat value"))
        }
    }

    // println!("{:?}", newargs);

    let newvec: Vec<&str> = newargs.iter().map(|s| {
        s.as_str()
    }).collect();
    let newvec = &newvec[..];

    // call zadd with newargs
    ctx.call("zadd", newvec)
}

///
/// H3.ADDBYINDEX key h3idx name [h3idx2 name2 ... h3idxN nameN]
///
/// this is an alternate to H3.ADD that takes an H3Index instead of lng/lat
///
/// NOTE: h3idx must have resolution 15 to be considered valid, otherwise an error is raised
///
fn h3addbyindex_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    if args.len() < 4 || args.len() % 2 != 0 {
        return Err(RedisError::from(
            "syntax error. Try H3.ADDBYINDEX key [h3idx1] [name1] [h3idx2] [name2] ... "
        ));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let elements: usize = args.len() / 2;
    let argc: usize = 2+elements*2; /* ZADD key score elem ... */

    let mut newargs: Vec<String> = Vec::with_capacity(argc);
    newargs.push(key);

    while args.len() > 0 {
        let h3key = args.next_string()?;
        let name = args.next_string()?;

        match str_to_h3(&h3key) {
            Ok(h3idx) => {
                // verify resolution 15
                if h3idx.resolution() != MAX_RESOLUTION {
                    return Err(RedisError::from("Invalid h3idx resolution (must be 15)"))
                }
                // this line is not optimal, the line after would be put member is not pub
                let h3ll = u64::from_str_radix(h3idx.to_string().as_str(), 16).unwrap();
                // let H3Index(h3ll) = h3idx;
                let score = h3ll_to_score(h3ll);

                newargs.push(format!("{}", score));
                newargs.push(name.clone());
            },
            Err(_err) => return Err(RedisError::from("Invalid h3idx value"))
        }
    }

    // println!("{:?}", newargs);

    let newvec: Vec<&str> = newargs.iter().map(|s| {
        s.as_str()
    }).collect();
    let newvec = &newvec[..];

    // call zadd with newargs
    ctx.call("zadd", newvec)
}

///
/// get_zscores - private function to get zscores for a list of zset elements for key, users of
/// this function will be responsible for determining whether scores are (convertible to) valid
/// H3Index values
///
fn get_zscores(ctx: &Context, key: String, elems: Vec<String>) -> RedisResult {
    let mut scores: Vec<RedisValue> = Vec::with_capacity(elems.len());

    let mut members = elems.into_iter();
    while members.len() > 0 {
        let elem = members.next_string()?;
        match ctx.call("zscore", &[&key, &elem]) {
            Ok(v) => {
                match v {
                    RedisValue::Float(f) => {
                        scores.push(f.into());
                    },
                    RedisValue::SimpleString(s) => {
                        let score: f64 = s.parse::<f64>().unwrap();
                        scores.push(score.into());
                    },
                    // this means an entry wasn't found for the elem, ignoring for now
                    RedisValue::Null => scores.push(RedisValue::Null),
                    _ => {
                        println!("v: {:?}", v);
                        return Err(RedisError::from("Unexpected type (SimpleString or Null)"))
                    }
                }
            },
            Err(err) => return Err(err)
        }
    }

    Ok(scores.into())
}

/// call this function to get zset scores converted to H3Index instances (calls get_zscores
/// and does the conversion)
fn get_zscores_as_h3_indices(ctx: &Context, key: String, elems: Vec<String>) -> Result<Vec<Option<H3Index>>,RedisError> {
    let mut opt_err: Option<&str> = None;
    let h3_indices: Vec<Option<H3Index>> = match get_zscores(&ctx, key, elems) {
        Ok(RedisValue::Array(scores)) => {
            scores.iter()
                .map(|s| {
                    match s {
                        RedisValue::Float(f) => {
                            let h3ll = score_to_h3ll(*f);
                            match H3Index::new(h3ll) {
                                Ok(h3idx) => Some(h3idx),
                                Err(_err) => {
                                    opt_err = Some("Invalid h3idx value");
                                    None
                                }
                            }
                        },
                        _ => None,
                    }
                }).collect()
        },
        Ok(v) => {
            println!("v: {:?}", v);
            return Err(RedisError::from("Unexpected type (not Array)"));
        },
        Err(err) => return Err(err)
    };
    if opt_err.is_some() {
        return Err(RedisError::from(opt_err.unwrap()))
    }
    Ok(h3_indices)
}

///
/// H3.INDEX key elem1 elem2 ... elemN
///
/// Returns an array with H3Index representations of the positions of
/// the specified elements ("translation" of geohashCommand)
///
fn h3index_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let args: Vec<String> = args.collect();

    match get_zscores_as_h3_indices(&ctx, key, args) {
        Ok(scores) => {
            let h3indices: Vec<RedisValue> = scores.iter().map(|opt_idx| {
                match opt_idx {
                    Some(h3idx) => h3idx.to_string().into(),
                    None => RedisValue::Null
                }
            }).collect();
            Ok(h3indices.into())
        }
        Err(err) => Err(err),
    }
}

///
/// H3.POS key elem1 elem2 ... elemN
///
/// Returns an array with lng/lat arrays of the positions of
/// the specified elements ("translation" of geoposCommand)
///
fn h3pos_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let args: Vec<String> = args.collect();

    match get_zscores_as_h3_indices(&ctx, key, args) {
        Ok(scores) => {
            let h3indices: Vec<RedisValue> = scores.iter().map(|opt_idx| {
                match opt_idx {
                    Some(h3idx) => {
                        let coord = h3idx.to_geo();
                        vec![coord.lon.to_string(), coord.lat.to_string()].into()
                    },
                    None => RedisValue::Null
                }
            }).collect();
            Ok(h3indices.into())
        }
        Err(err) => Err(err),
    }
}

///
/// H3.CELL key h3idx [WITHINDICES] [LIMIT offset count]
///
/// Returns an array of the elements in the zset that are contained within the H3 cell
/// for the given index
///
fn h3cell_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let syntax_err_msg = "syntax error. Try H3.CELL key h3idx [WITHINDICES] [LIMIT offset count]";
    if args.len() < 3 {
        return Err(RedisError::from(syntax_err_msg));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let h3key = args.next_string()?;
    let mut withindices = false;
    let mut limit = false;
    let mut offset = 0;
    let mut count = 0;

    while let Ok(arg) = args.next_string() {
        match arg.to_uppercase().as_str() {
            "WITHINDICES" => {
                withindices = true;
            }
            "LIMIT" => {
                limit = true;
                if args.len() < 2 {
                    return Err(RedisError::from(syntax_err_msg));
                }
                offset = args.next_i64()?;
                count = args.next_i64()?;
            }
            _ => {
                return Err(RedisError::from(syntax_err_msg));
            }
        }
    }

    let h3idx = match str_to_h3(&h3key) {
        Ok(h3idx) => h3idx,
        Err(_err) => return Err(RedisError::from("Invalid h3idx value"))
    };

    // it would be better to get the u64 value from h3idx (commented line under next),
    // but member is not pub
    let h3ll = u64::from_str_radix(h3idx.to_string().as_str(), 16).unwrap();
    // let H3Index(h3ll) = h3idx;

    let min_child = index_min_child(h3ll);
    let min_score = h3ll_to_score(min_child);
    let min_score = format!("{}", min_score);

    let max_child = index_max_child(h3ll);
    let max_score = h3ll_to_score(max_child);
    let max_score = format!("{}", max_score);

    let mut newargs: Vec<String> = vec![key, min_score, max_score];
    if withindices {
        newargs.push(String::from("withscores"));
    }
    if limit {
        newargs.push(String::from("limit"));
        newargs.push(format!("{}", offset.clone()));
        newargs.push(format!("{}", count.clone()));
    }

    let newargs: Vec<&str> = newargs.iter().map(|s| {
        s.as_str()
    }).collect();
    let newargs = &newargs[..];

    match ctx.call("zrangebyscore", newargs) {
        Ok(v) => {
            match &v {
                RedisValue::Array(elems) => {
                    let mut newvec: Vec<RedisValue> = Vec::with_capacity(elems.len());
                    let mut i = 0;
                    while i < elems.len() {
                        let elem: &String = match &elems[i] {
                            RedisValue::SimpleString(s) => s,
                            _ => {
                                println!("v: {:?}", &v);
                                return Err(RedisError::from("Unexpected type (not SimpleString)"))
                            }
                        };
                        if withindices && i % 2 != 0 {
                            let score: f64 = elem.parse::<f64>().unwrap();
                            let h3ll = score_to_h3ll(score);

                            match H3Index::new(h3ll) {
                                Ok(h3idx) => {
                                    newvec.push(h3idx.to_string().into())
                                },
                                Err(_err) => return Err(RedisError::from("Invalid h3idx value"))
                            }
                        } else {
                            newvec.push(elem.into())
                        }

                        i += 1;
                    }
                    Ok(newvec.into())
                },
                // this means an entry wasn't found for the elem, ignoring for now
                RedisValue::Null => Ok(RedisValue::Null),
                _ => {
                    println!("v: {:?}", v);
                    return Err(RedisError::from("Unexpected type (not Array or Null)"))
                }
            }
        },
        Err(err) => return Err(err)
    }
}

///
/// H3.COUNT key h3idx
///
/// this is a translation of the ZCOUNT command that takes an H3Index and returns the number of
/// elements contained within the H3 cell for the given index
///
fn h3count_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    if args.len() != 3 {
        return Err(RedisError::from("syntax error. Try H3.COUNT key h3idx"));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let h3key = args.next_string()?;

    let h3idx = match str_to_h3(&h3key) {
        Ok(h3idx) => h3idx,
        Err(_err) => return Err(RedisError::from("Invalid h3idx value"))
    };

    // it would be better to get the u64 value from h3idx (commented line under next),
    // but member is not pub
    let h3ll = u64::from_str_radix(h3idx.to_string().as_str(), 16).unwrap();
    // let H3Index(h3ll) = h3idx;

    let min_child = index_min_child(h3ll);
    let min_score = h3ll_to_score(min_child);
    let min_score = format!("{}", min_score);

    let max_child = index_max_child(h3ll);
    let max_score = h3ll_to_score(max_child);
    let max_score = format!("{}", max_score);

    let newargs: Vec<&str> = vec![&key, &min_score, &max_score];
    let newargs = &newargs[..];

    ctx.call("zcount", newargs)
}

/// a translation of the GEODIST command
fn h3dist_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err("Command not implemented".into())
}

/// a translation of the GEORADIUS command
fn h3radius_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err("Command not implemented".into())
}

/// a translation of the GEORADIUSBYMEMBER command
fn h3radiusbyindex_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err("Command not implemented".into())
}

//////////////////////////////////////////////////////

pub extern "C" fn init(_raw_ctx: *mut rawmod::RedisModuleCtx) -> c_int {
    0
}

redis_module! {
    name: "h3",
    version: 1,
    data_types: [],
    init: init,
    commands: [
        ["h3.status", h3status_command, "", 0, 0, 0],
        ["h3.add", h3add_command, "write deny-oom", 1, 1, 1],
        ["h3.addbyindex", h3addbyindex_command, "write deny-oom", 1, 1, 1],
        ["h3.index", h3index_command, "readonly", 1, 1, 1],
        ["h3.pos", h3pos_command, "readonly", 1, 1, 1],
        ["h3.cell", h3cell_command, "readonly", 1, 1, 1],
        ["h3.count", h3count_command, "readonly", 1, 1, 1],
        ["h3.dist", h3dist_command, "readonly", 1, 1, 1],
        ["h3.radius", h3radius_command, "readonly", 1, 1, 1],
        ["h3.radiusbyindex", h3radiusbyindex_command, "readonly", 1, 1, 1],
    ],
}

//////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use redis_module::RedisValue;

    use super::*;

    fn run_status() -> RedisResult {
        h3status_command(
            &Context::dummy(),
            vec![],
        )
    }

    #[test]
    fn test_status() {
        let result = run_status();

        match result {
            Ok(RedisValue::SimpleString(s)) => {
                assert!(s.as_str() == "Ok");
            }
            _ => assert!(false, "Bad result: {:?}", result),
        }
    }
}
