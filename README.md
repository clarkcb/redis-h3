# redis-h3

redis-h3 is a Redis module, implemented in Rust, that adds support for Uber's H3 geospatial indexing system. The added
commands (all prefixed with `"H3."`) are somewhat modeled after the Redis core Geo commands and perform somewhat similar
functions. Both sets of commands make use of the underlying Sorted Set data type, but in the case of H3,
the scores are 64-bit double values corresponding to H3 indices.

The module depends on the following crates:

* [redismodule-rs](https://github.com/clarkcb/redismodule-rs) - provides an API framework for writing Redis modules in
  idiomatic Rust (forked from [RedisLabsModules/redismodule-rs](https://github.com/RedisLabsModules/redismodule-rs))
* [h3-rs](https://github.com/clarkcb/h3-rs) - provides Rust bindings for
  [Uber's H3 C library](https://uber.github.io/h3/) (forked from [jeromefroe/h3-rs](https://github.com/jeromefroe/h3-rs))

## Quick Overview

### Conventions / Considerations
* The resolution for H3 indices to be used as Sorted Set score values will always be the highest (15). This ensures
  correct determination of corresponding cells at all resolutions, and also provides best results for distance
  calculations, etc.
* Since elements' scores will be highest resolution H3 indices, their locations will be accurate to within 1m squared.

### Commands
These are the currently implemented commands:
* __`H3.STATUS`__ - get status of H3 module (can be used to determine if the module is loaded)
* __`H3.ADD [key] [lng1] [lat1] [elem1] ... [lngN] [latN] [elemN]`__ - similar to the `GEOADD` command, but add elements
  for H3 indices calculated from the given lng/lat values
* __`H3.ADDBYINDEX [key] [h3idx1] [elem1] ... [h3idxN] [elemN]`__ - (unique to H3 module) add entries by H3 index
  instead of lng/lat position
  NOTE: the indices are required to be of highest resolution (15); if not an error will be returned
* __`H3.INDEX [key] [elem1] ... [elemN]`__ - similar to `GEOHASH` command, returns the H3 index for one or more given
  elements
* __`H3.POS [key] [elem1] ... [elemN]`__ - similar to `GEOPOS` command, but returns the centroid lng and lat for the
  H3 indices for the given elements

These are some other possible commands to implement:
* __`H3.ININDEX [key] [h3idx]`__ - get list of elements contained in the cell of the given H3 index (any resolution
  should be allowed for H3 keys for this command)
* __`H3.REMBYINDEX [key] [h3idx1] ... [h3idxN]`__ - remove the elems matching any of the given H3 indices

These are the Geo commands that there are currently no counterparts for (and not sure if there will be):
* __`GEODIST [key] [elem1] [elem2] ...`__ - return the distance between two elements in the geospatial index
* __`GEORADIUS [key] [lng] [lng] [radius] ...`__ - return the elements that are within the border of the area
  specified with the center lng/lat position and the max distance from the center (radius)
* __`GEORADIUSBYMEMBER [key] [elem] [radius] ...`__ - return the elements that are within the border of the area
  specified with element's position and the max distance from the position (radius)

## Setup
1. [Install Redis](https://redis.io/topics/quickstart) - the site suggests installing from source to ensure the latest,
   but I use Homebrew on my Mac:
   ```
   $ brew install redis
   ```
2. If running as a service, stop it - if installed using Homebrew:
   ```
   $ brew services list
   ```
   and if running:
   ```
   $ brew services stop redis
   ```
3. [Install Rust](https://www.rust-lang.org/tools/install) - `rustup` is the preferred tool to install, but other
   options are available
4. Git clone redis-h3
5. Build redis-h3
   ```
   $ cargo build
   ```

## Running
Run Redis server manually, loading RedisH3 module, for example:
```
$ redis-server --loadmodule target/debug/libredish3.dylib
```

You should see output similar to this when Redis starts up:
```
88337:C 08 Jun 2020 15:25:15.449 # oO0OoO0OoO0Oo Redis is starting oO0OoO0OoO0Oo
88337:C 08 Jun 2020 15:25:15.449 # Redis version=6.0.4, bits=64, commit=00000000, modified=0, pid=88337, just started
88337:C 08 Jun 2020 15:25:15.449 # Configuration loaded
88337:M 08 Jun 2020 15:25:15.451 * Increased maximum number of open files to 10032 (it was originally set to 256).
                _._                                                  
           _.-``__ ''-._                                             
      _.-``    `.  `_.  ''-._           Redis 6.0.4 (00000000/0) 64 bit
  .-`` .-```.  ```\/    _.,_ ''-._                                   
 (    '      ,       .-`  | `,    )     Running in standalone mode
 |`-._`-...-` __...-.``-._|'` _.-'|     Port: 6379
 |    `-._   `._    /     _.-'    |     PID: 88337
  `-._    `-._  `-./  _.-'    _.-'                                   
 |`-._`-._    `-.__.-'    _.-'_.-'|                                  
 |    `-._`-._        _.-'_.-'    |           http://redis.io        
  `-._    `-._`-.__.-'_.-'    _.-'                                   
 |`-._`-._    `-.__.-'    _.-'_.-'|                                  
 |    `-._`-._        _.-'_.-'    |                                  
  `-._    `-._`-.__.-'_.-'    _.-'                                   
      `-._    `-.__.-'    _.-'                                       
          `-._        _.-'                                           
              `-.__.-'                                               

88337:M 08 Jun 2020 15:25:15.453 # Server initialized
88337:M 08 Jun 2020 15:25:15.456 * Module 'h3' loaded from ./target/debug/libredish3.dylib
88337:M 08 Jun 2020 15:25:15.457 * Loading RDB produced by version 6.0.3
88337:M 08 Jun 2020 15:25:15.457 * RDB age 1292383 seconds
88337:M 08 Jun 2020 15:25:15.457 * RDB memory usage when created 0.97 Mb
88337:M 08 Jun 2020 15:25:15.457 * DB loaded from disk: 0.001 seconds
88337:M 08 Jun 2020 15:25:15.457 * Ready to accept connections
```

## Example Session
Here's an example `redis-cli` session that compares Geo and H3 commands:

```
$ redis-cli
127.0.0.1:6379> H3.STATUS
"Ok"
127.0.0.1:6379> GEOADD GEOSicily 13.361389 38.115556 "Palermo" 15.087269 37.502669 "Catania"
(integer) 2
127.0.0.1:6379> ZSCAN GEOSicily 0
1) "0"
2) 1) "Palermo"
   2) "3479099956230698"
   3) "Catania"
   4) "3479447370796909"
127.0.0.1:6379> H3.ADD H3Sicily 13.361389 38.115556 "Palermo" 15.087269 37.502669 "Catania"
(integer) 2
127.0.0.1:6379> ZSCAN H3Sicily 0
1) "0"
2) 1) "Palermo"
   2) "538352348825157"
   3) "Catania"
   4) "1112003081711909"
127.0.0.1:6379> H3.ADDBYINDEX H3Sicily 8f1e9a0ec840645 "Palermo-idx" 8f3f35c64acb125 "Catania-idx"
(integer) 2
127.0.0.1:6379> ZSCAN H3Sicily 0
1) "0"
2) 1) "Palermo"
   2) "538352348825157"
   3) "Palermo-idx"
   4) "538352348825157"
   5) "Catania"
   6) "1112003081711909"
   7) "Catania-idx"
   8) "1112003081711909"
127.0.0.1:6379> GEOHASH GEOSicily "Palermo" "Catania"
1) "sqc8b49rny0"
2) "sqdtr74hyu0"
127.0.0.1:6379> H3.INDEX H3Sicily "Palermo" "Catania"
1) "8f1e9a0ec840645"
2) "8f3f35c64acb125"
127.0.0.1:6379> GEOPOS GEOSicily "Palermo" "Catania"
1) 1) "13.36138933897018433"
   2) "38.11555639549629859"
2) 1) "15.08726745843887329"
   2) "37.50266842333162032"
127.0.0.1:6379> H3.POS H3Sicily "Palermo" "Catania"
1) 1) "13.361384873217883"
   2) "38.115552632253305"
2) 1) "15.087270305767186"
   2) "37.50266586290179"
```

To verify the values, here are some H3 CLI commands with output to compare against:
```
$ h3ToGeo -i 8f1e9a0ec840645
38.1155526323 13.3613848732
$ h3ToGeo -i 8f3f35c64acb125
37.5026658629 15.0872703058
$ geoToH3 --lat 38.115556 --lon 13.361389 -r 15
8f1e9a0ec840645
$ geoToH3 --lat 37.502669 --lon 15.087269 -r 15
8f3f35c64acb125
```