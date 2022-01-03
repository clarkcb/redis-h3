#[macro_use]
extern crate redis_module;

use std::os::raw::c_int;

use h3_rs::{GeoCoord, H3Index};
use redis_module::{NextArg, raw as rawmod};
use redis_module::{Context, RedisError, RedisResult, RedisValue};

use crate::geoutil::{geohash_get_distance};
use crate::h3util::{h3ll_to_score, index_max_child, index_min_child,
                    MAX_RESOLUTION, score_to_h3ll, str_to_h3};

mod h3util;
mod geoutil;

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
        return Err(RedisError::Str(
            "syntax error. Try H3.ADD key [lng1] [lat1] [name1] [lng2] [lat2] [name2] ... "
        ));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let elements: usize = args.len() / 3;
    let argc: usize = 2 + elements * 2; /* ZADD key score elem ... */

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
            _ => return Err(RedisError::Str("Invalid lng or lat value"))
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
        return Err(RedisError::Str(
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
                    return Err(RedisError::Str("Invalid h3idx resolution (must be 15)"))
                }
                // this line is not optimal, the line after would be put member is not pub
                let h3ll = u64::from_str_radix(h3idx.to_string().as_str(), 16).unwrap();
                // let H3Index(h3ll) = h3idx;
                let score = h3ll_to_score(h3ll);

                newargs.push(format!("{}", score));
                newargs.push(name.clone());
            },
            Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
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
fn get_zscores(ctx: &Context, key: &String, elems: Vec<String>) -> RedisResult {
    let mut scores: Vec<RedisValue> = Vec::with_capacity(elems.len());

    let mut members = elems.into_iter();
    while members.len() > 0 {
        let elem = members.next_string()?;
        match ctx.call("zscore", &[key, &elem]) {
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
                        return Err(RedisError::Str("Unexpected type (SimpleString or Null)"))
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
fn get_zscores_as_h3_indices(ctx: &Context, key: &String, elems: Vec<String>) -> Result<Vec<Option<H3Index>>,RedisError> {
    let mut opt_err: Option<&str> = None;
    let h3_indices: Vec<Option<H3Index>> = match get_zscores(&ctx, &key, elems) {
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
            return Err(RedisError::Str("Unexpected type (not Array)"));
        },
        Err(err) => return Err(err)
    };
    if opt_err.is_some() {
        return Err(RedisError::Str(opt_err.unwrap()))
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

    match get_zscores_as_h3_indices(&ctx, &key, args) {
        Ok(vec_opt_h3indices) => {
            let h3indices: Vec<RedisValue> = vec_opt_h3indices.iter().map(|opt_idx| {
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
/// Returns an array with lng/lat arrays of the centroids of H3 indices
/// for the specified elements ("translation" of geoposCommand)
///
fn h3pos_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let args: Vec<String> = args.collect();

    match get_zscores_as_h3_indices(&ctx, &key, args) {
        Ok(vec_opt_h3indices) => {
            let h3pos: Vec<RedisValue> = vec_opt_h3indices.iter().map(|opt_idx| {
                match opt_idx {
                    Some(h3idx) => {
                        let coord = h3idx.to_geo();
                        vec![coord.lon.to_string(), coord.lat.to_string()].into()
                    },
                    None => RedisValue::Null
                }
            }).collect();
            Ok(h3pos.into())
        }
        Err(err) => Err(err),
    }
}

///
/// get_cell_members
///
/// Takes an H3 key (cell or index as string) and optional limit values and returns
/// all elems whose indices are children of the given H3 key
///
fn get_cell_members(ctx: &Context, key: &String, h3idx: &H3Index, withindices: bool, limit: bool,
                    offset: i64, count: i64) -> RedisResult {
    // let h3idx = match str_to_h3(&h3key) {
    //     Ok(h3idx) => h3idx,
    //     Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
    // };

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

    let mut newargs: Vec<String> = vec![key.clone(), min_score, max_score];
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
                                return Err(RedisError::Str("Unexpected type (not SimpleString)"))
                            }
                        };
                        if withindices && i % 2 != 0 {
                            let score: f64 = elem.parse::<f64>().unwrap();
                            let h3ll = score_to_h3ll(score);

                            match H3Index::new(h3ll) {
                                Ok(h3idx) => {
                                    newvec.push(h3idx.to_string().into())
                                },
                                Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
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
                    return Err(RedisError::Str("Unexpected type (not Array or Null)"))
                }
            }
        },
        Err(err) => return Err(err)
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
        return Err(RedisError::Str(syntax_err_msg));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let h3key = args.next_string()?;
    let mut withindices = false;
    let mut limit = false;
    let mut offset = 0;
    let mut count = 0;

    let h3idx = match str_to_h3(&h3key) {
        Ok(h3idx) => h3idx,
        Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
    };

    while let Ok(arg) = args.next_string() {
        match arg.to_uppercase().as_str() {
            "WITHINDICES" => {
                withindices = true;
            }
            "LIMIT" => {
                limit = true;
                if args.len() < 2 {
                    return Err(RedisError::Str(syntax_err_msg));
                }
                offset = args.next_i64()?;
                count = args.next_i64()?;
            }
            _ => {
                return Err(RedisError::Str(syntax_err_msg));
            }
        }
    }

    get_cell_members(ctx, &key, &h3idx, withindices, limit, offset, count)
}

///
/// H3.COUNT key h3idx
///
/// this is a translation of the ZCOUNT command that takes an H3Index and returns the number of
/// elements contained within the H3 cell for the given index
///
fn h3count_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    if args.len() != 3 {
        return Err(RedisError::Str("syntax error. Try H3.COUNT key h3idx"));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let h3key = args.next_string()?;

    let h3idx = match str_to_h3(&h3key) {
        Ok(h3idx) => h3idx,
        Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
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

///
/// H3.SCAN key cursor [MATCH pattern] [COUNT count]
///
/// this is a translation of the ZSCAN command, but instead of returning elements with scores,
/// it returns elements with H3 indices
///
fn h3scan_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let syntax_err_msg = "syntax error. Try H3.SCAN key cursor [MATCH pattern] [COUNT count]";
    if args.len() < 3 {
        return Err(RedisError::Str(syntax_err_msg));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let cursor = match args.next_i64() {
        Ok(c) => c,
        Err(_err) => return Err(RedisError::Str("invalid cursor"))
    };
    let mut match_pattern: Option<String> = None;
    let mut count: Option<i64> = None;

    while let Ok(arg) = args.next_string() {
        match arg.to_uppercase().as_str() {
            "MATCH" => match_pattern = Some(args.next_string()?),
            "COUNT" => count = Some(args.next_i64()?),
            _ => {
                return Err(RedisError::Str(syntax_err_msg));
            }
        }
    }

    let mut newargs: Vec<String> = vec![key, cursor.to_string()];
    if match_pattern.is_some() {
        newargs.push(String::from("match"));
        newargs.push(match_pattern.unwrap());
    }
    if count.is_some() {
        newargs.push(String::from("count"));
        newargs.push(count.unwrap().to_string());
    }

    let newargs: Vec<&str> = newargs.iter().map(|s| {
        s.as_str()
    }).collect();
    let newargs = &newargs[..];

    match ctx.call("zscan", newargs) {
        Ok(v) => match &v {
            RedisValue::Array(zscan_result) => {
                let mut h3scan_result: Vec<RedisValue> = Vec::with_capacity(zscan_result.len());
                let mut zscan_result = zscan_result.into_iter();
                let next_cursor = match zscan_result.next() {
                    Some(RedisValue::SimpleString(s)) => s,
                    _ => {
                        return Err(RedisError::Str("Unexpected type (not SimpleString)"))
                    }
                };
                h3scan_result.push(next_cursor.into());
                match zscan_result.next() {
                    Some(RedisValue::Array(elems_with_scores)) => {
                        let mut elems_with_indices: Vec<RedisValue> =
                            Vec::with_capacity(elems_with_scores.len());

                        let mut i = 0;
                        while i < elems_with_scores.len() {
                            let elem: &String = match &elems_with_scores[i] {
                                RedisValue::SimpleString(s) => s,
                                _ => {
                                    return Err(RedisError::Str("Unexpected type (not SimpleString)"))
                                }
                            };
                            if i % 2 != 0 {
                                let score: f64 = elem.parse::<f64>().unwrap();
                                let h3ll = score_to_h3ll(score);

                                match H3Index::new(h3ll) {
                                    Ok(h3idx) => {
                                        elems_with_indices.push(h3idx.to_string().into())
                                    },
                                    Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
                                }
                            } else {
                                elems_with_indices.push(elem.into())
                            }

                            i += 1;
                        }
                        h3scan_result.push(elems_with_indices.into());
                    },
                    _ => return Err(RedisError::Str("Unexpected type (not Array)"))
                }

                Ok(h3scan_result.into())
            }
            // this means an entry wasn't found for the elem, ignoring for now
            RedisValue::Null => Ok(RedisValue::Null),
            _ => {
                println!("v: {:?}", &v);
                return Err(RedisError::Str("Unexpected type (not Array or Null)"))
            }
        },
        Err(err) => return Err(err)
    }
}

fn unit_str_to_conversion(unit: &String) -> Result<f64,RedisError> {
    let conversion = match unit.to_uppercase().as_str() {
        "M" => 1.0,
        "KM" => 1000.0,
        "FT" => 0.3048,
        "MI" => 1609.34,
        _ => -1.0
    };
    if conversion > -1.0 {
        Ok(conversion)
    } else {
        Err(RedisError::Str("unsupported unit provided. please use m, km, ft, mi"))
    }
}

///
/// H3.DIST key elem1 elem2 [unit]
///
/// this is a translation of the GEODIST command
///
fn h3dist_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let syntax_err_msg = "syntax error. Try H3.DIST key elem1 elem2 [unit]";
    if args.len() < 4 {
        return Err(RedisError::Str(syntax_err_msg));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;
    let elem1 = args.next_string()?;
    let elem2 = args.next_string()?;
    let mut to_meter: f64 = 1.0;

    if let Ok(unit) = args.next_string() {
        match unit_str_to_conversion(&unit) {
            Ok(conversion) => to_meter = conversion,
            Err(err) => {
                return Err(err);
            }
        }
    }

    match get_zscores_as_h3_indices(&ctx, &key, vec![elem1, elem2]) {
        Ok(vec_opt_h3indices) => {
            let dist: f64 = match (vec_opt_h3indices.get(0), vec_opt_h3indices.get(1)) {
                (Some(Some(h3idx1)), Some(Some(h3idx2))) => {
                    let coord1: GeoCoord = h3idx1.to_geo();
                    let coord2: GeoCoord = h3idx2.to_geo();
                    geohash_get_distance(coord1.lon, coord1.lat, coord2.lon, coord2.lat) / to_meter
                },
                _ => -1.0
            };
            if dist > -1.0 {
                Ok(format!("{:.4}", dist).into())
            } else {
                Err(RedisError::Str("error trying to get distance"))
            }
        },
        Err(err) => Err(err)
    }
}

///
/// H3.REMBYINDEX key h3idx1 ... [h3idxN]
///
/// remove elements that match a given H3 index
///
fn h3rembyindex_command(ctx: &Context, args: Vec<String>) -> RedisResult {
    let syntax_err_msg = "syntax error. Try H3.REMBYINDEX key h3idx1 ... [h3idxN]";
    if args.len() < 3 {
        return Err(RedisError::Str(syntax_err_msg));
    }

    let mut args = args.into_iter().skip(1);
    let key = args.next_string()?;

    let zremargc: usize = 1 + args.len(); /* ZREM key elem ... */

    let mut zremargs: Vec<String> = Vec::with_capacity(zremargc);
    zremargs.push(key.clone());

    while let Ok(h3key) = args.next_string() {

        let h3idx = match str_to_h3(&h3key) {
            Ok(h3idx) => h3idx,
            Err(_err) => return Err(RedisError::Str("Invalid h3idx value"))
        };
    
        match get_cell_members(ctx, &key, &h3idx, false, false, 0, 0) {
            Ok(v) => {
                match &v {
                    RedisValue::Array(elems) => {
                        if !elems.is_empty() {
                            let mut i = 0;
                            while i < elems.len() {
                                match &elems[i] {
                                    RedisValue::SimpleString(name) => {
                                        zremargs.push(name.to_owned());
                                    },
                                    RedisValue::BulkString(name) => {
                                        zremargs.push(name.to_owned());
                                    },
                                    _ => {
                                        println!("v: {:?}", &v);
                                        return Err(RedisError::Str("Unexpected types (not SimpleString)"))
                                    }
                                }
                                i += 1;
                            }
                        }
                    },
                    // this means an entry wasn't found for the elem, do nothing for now
                    RedisValue::Null => {
                        println!("v: {:?}", v);
                    },
                    _ => {
                        println!("v: {:?}", v);
                        return Err(RedisError::Str("Unexpected type (not Array or Null)"))
                    }
                }
            },
            Err(err) => return Err(err)
        }
    }

    if zremargs.len() > 1 {
        let zremargs: Vec<&str> = zremargs.iter().map(|s| {
            s.as_str()
        }).collect();
        let zremargs = &zremargs[..];
    
        // call zadd with zremargs
        ctx.call("zrem", zremargs)
    } else {
        let zero: i64 = 0;
        Ok(zero.into())
    }
}

/// a translation of the GEORADIUS command
fn h3radius_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err(RedisError::Str("Command not implemented"))
}

/// a translation of the GEORADIUSBYMEMBER command
fn h3radiusbyindex_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err(RedisError::Str("Command not implemented"))
}

/// a translation of the GEOSEARCH command
fn h3search_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err(RedisError::Str("Command not implemented"))
}

/// a translation of the GEOSEARCHSTORE command
fn h3searchstore_command(_ctx: &Context, _args: Vec<String>) -> RedisResult {
    Err(RedisError::Str("Command not implemented"))
}

//////////////////////////////////////////////////////

// pub extern "C" fn init(_raw_ctx: *mut rawmod::RedisModuleCtx) -> c_int {
//     0
// }

redis_module! {
    name: "h3",
    version: 1,
    data_types: [],
    // init: init,
    commands: [
        ["h3.status", h3status_command, "", 0, 0, 0],
        ["h3.add", h3add_command, "write deny-oom", 1, 1, 1],
        ["h3.addbyindex", h3addbyindex_command, "write deny-oom", 1, 1, 1],
        ["h3.index", h3index_command, "readonly", 1, 1, 1],
        ["h3.pos", h3pos_command, "readonly", 1, 1, 1],
        ["h3.cell", h3cell_command, "readonly", 1, 1, 1],
        ["h3.count", h3count_command, "readonly", 1, 1, 1],
        ["h3.dist", h3dist_command, "readonly", 1, 1, 1],
        ["h3.rembyindex", h3rembyindex_command, "write", 1, 1, 1],
        ["h3.radius", h3radius_command, "readonly", 1, 1, 1],
        ["h3.radiusbyindex", h3radiusbyindex_command, "readonly", 1, 1, 1],
        ["h3.scan", h3scan_command, "readonly", 1, 1, 1],
        ["h3.search", h3search_command, "readonly", 1, 1, 1],
        ["h3.searchstore", h3searchstore_command, "readonly", 1, 1, 1],
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
