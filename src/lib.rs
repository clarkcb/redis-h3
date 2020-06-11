#[macro_use]
extern crate redis_module;

use redis_module::native_types::RedisType;
use redis_module::{raw as rawmod, NextArg};
use redis_module::{Context, RedisError, RedisResult, RedisString, RedisValue, REDIS_OK};

use std::os::raw::c_int;

use h3_rs::{GeoCoord, H3Index};
use std::convert::TryInto;

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

    const DEFAULT_RESOLUTION: i32 = 10;

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let elements: usize = args.len() / 3;
    let argc: usize = 2+elements*2; /* ZADD key score elem ... */

    let mut newargs: Vec<String> = Vec::with_capacity(argc);
    newargs.push(key);

    /* Create the argument vector to call ZADD in order to add all
     * the score,value pairs to the requested zset, where score is actually
     * an encoded version of lat,long. */
    while (args.len() > 0) {
        match (args.next_f64(), args.next_f64()) {
            (Ok(lng), Ok(lat)) => {
                let name = args.next_string()?;
                let coord: GeoCoord = GeoCoord::new(lat, lng);
                // TODO: determine default resolution, or add res arg to command
                let h3 = coord.to_h3(DEFAULT_RESOLUTION).unwrap();
                let h3dbl = u64::from_str_radix(h3.to_string().as_str(), 16).unwrap();

                newargs.push(format!("{:?}", h3dbl));
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

    while (args.len() > 0) {
        let h3key = args.next_string()?;
        let name = args.next_string()?;

        match H3Index::from_str(&h3key) {
            Ok(h3index) => {
                let h3dbl = u64::from_str_radix(h3key.as_str(), 16).unwrap();
                newargs.push(format!("{:?}", h3dbl));
                newargs.push(name.clone());
            },
            Err(err) => return Err(RedisError::from("Invalid h3idx value"))
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
/// get_zscores - private function to get zscores for a list of zset elements for key
///
fn get_zscores(ctx: &Context, key: String, elems: Vec<String>) -> RedisResult {
    let mut scores: Vec<RedisValue> = Vec::with_capacity(elems.len());

    let mut members = elems.into_iter();
    while (members.len() > 0) {
        let elem = members.next_string()?;
        match ctx.call("zscore", &[&key, &elem]) {
        // match ctx.call_to_int("zscore", &[&key, &elem]) {
        // match ctx.get_zscore(&key, &elem) {
            Ok(v) => {
                println!("v: {:?}", v);
                match v {
                    RedisValue::Float(mut f) => {
                        println!("f: {}", f);
                        println!("f as u64: {}", f as u64);
                        match H3Index::new(f as u64) {
                            Ok(h3index) => {
                                println!("h3index (from Float): {}", h3index);
                                scores.push(f.into());
                            },
                            Err(err) => {
                                println!("err: {:?}", err);
                            }
                        }
                    }
                    RedisValue::Integer(mut i) => {
                        println!("i: {}", i);
                        match H3Index::new(i as u64) {
                            Ok(h3index) => {
                                println!("h3index (from Integer): {}", h3index);
                                scores.push(i.into());
                            },
                            Err(err) => {
                                println!("err: {:?}", err);
                            }
                        }
                    },
                    RedisValue::SimpleString(mut s) => {
                        let score = match s.find('.') {
                            Some(d) => {
                                let f = s.parse::<f64>().unwrap();
                                f as u64
                            },
                            None => s.parse::<u64>().unwrap()
                        };
                        // this is a terrible hack but seems to work (sort of)!!!
                        // TODO: retrieve the double directly from redis instead
                        //       of converting from string
                        match (H3Index::new(score), H3Index::new(score - 1)) {
                            (Err(_), Ok(h3index)) => {
                                // in this case we use the score - 1 value
                                // println!("h3index (-1): {}", h3index);
                                scores.push(((score - 1) as i64).into());
                            },
                            (Ok(h3index), _) => {
                                // println!("h3index (1): {}", h3index);
                                scores.push((score as i64).into());
                            },
                            _ => return Err(RedisError::from("invalid H3Index value"))
                        }
                    },
                    // this means an entry wasn't found for the elem, ignoring for now
                    RedisValue::Null => scores.push(RedisValue::Null),
                    _ => return Err(RedisError::from("v not an expected type (SimpleString or Null)"))
                }
            },
            Err(err) => return Err(RedisError::from("something went wrong"))
        }
    }

    Ok(scores.into())
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

    match get_zscores(&ctx, key, args) {
        Ok(RedisValue::Array(scores)) => {
            let h3indices: Vec<RedisValue> = scores.iter()
                .map(|s| {
                    println!("s: {:?}", s);
                    match s {
                        RedisValue::Integer(i) => {
                            let h3index = H3Index::new(i.to_owned() as u64).unwrap();
                            h3index.to_string().into()
                        },
                        _ => RedisValue::Null,
                    }
                }).collect();
            println!("{:?}", h3indices);
            Ok(h3indices.into())
        },
        Err(err) => Err(err),
        _ => return Err(RedisError::from("scores not an expected type: Array"))
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

    match get_zscores(&ctx, key, args) {
        Ok(RedisValue::Array(scores)) => {
            let h3indices: Vec<RedisValue> = scores.iter()
                .map(|s| {
                    println!("s: {:?}", s);
                    match s {
                        RedisValue::Integer(s) => {
                            let h3index = H3Index::new(s.to_owned() as u64).unwrap();
                            let coord = h3index.to_geo();
                            vec![coord.lon.to_string(), coord.lat.to_string()].into()
                        },
                        _ => RedisValue::Null,
                    }

                }).collect();
            // println!("{:?}", h3indices);
            Ok(h3indices.into())
        },
        Err(err) => Err(err),
        _ => return Err(RedisError::from("scores not an expected type: Array"))
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
    use super::*;
    use redis_module::RedisValue;

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
