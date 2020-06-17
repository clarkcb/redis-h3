#[macro_use]
extern crate redis_module;

use std::os::raw::c_int;

use h3_rs::{GeoCoord, H3Index};
use redis_module::{NextArg, raw as rawmod};
use redis_module::{Context, RedisError, RedisResult, RedisValue};

// all H3 indices used as scores must have this resolution
const RESOLUTION: i32 = 15;

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
    // dec: 4503599627370495
    // hex: 0x000FFFFFFFFFFFFF
    // bin: 0b0000_0000_0000_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111_1111
    let low52_mask: u64 = 0x000FFFFFFFFFFFFF;
    h3ll &= low52_mask; // Unset top 12 bits
    h3ll as f64
}

// convert zset score double to H3 long long
fn score_to_h3ll(score: f64) -> u64 {
    // dec: 644014746713980928
    // hex: 0x08F0000000000000
    // bin: 0b0000_1000_1111_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000_0000
    let high_mask: u64 = 0x08F0000000000000;
    let mut h3ll = score as u64;
    h3ll |= high_mask; // Set high mask bits
    h3ll
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
/// TODO: add RES N as optional arg to specify resolution (currently defaults to 10)
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
                // TODO: need to verify valid lng/lat (should probably happen in GeoCoord::new)
                let coord: GeoCoord = GeoCoord::new(lat, lng);
                let h3_from_coord = coord.to_h3(RESOLUTION).unwrap();
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

        match H3Index::from_str(&h3key) {
            Ok(h3idx) => {
                // verify resolution 15
                if h3idx.resolution() != RESOLUTION {
                    return Err(RedisError::from("Invalid h3idx resolution (must be 15)"))
                }
                let h3ll = u64::from_str_radix(h3key.as_str(), 16).unwrap();
                let score = h3ll_to_score(h3ll);

                newargs.push(format!("{}", score));
                newargs.push(name.clone());
            }
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
            vec![]
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
