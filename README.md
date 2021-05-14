# redis-h3

redis-h3 is a Redis module, implemented in Rust, that adds support for Uber's H3 geospatial
indexing system. The added commands (all prefixed with `"H3."`) are somewhat modeled after the
Redis core Geo commands and perform somewhat similar functions. Both sets of commands make use of
the underlying Sorted Set data type, but in the case of H3, the scores are 64-bit double values
corresponding to H3 indices.

The module depends on the following crates:

* [redismodule-rs](https://github.com/clarkcb/redismodule-rs) - provides an API framework for
  writing Redis modules in idiomatic Rust (forked from
  [RedisLabsModules/redismodule-rs](https://github.com/RedisLabsModules/redismodule-rs))
* [h3-rs](https://github.com/clarkcb/h3-rs) - provides Rust bindings for
  [Uber's H3 C library](https://uber.github.io/h3/)
  (forked from [jeromefroe/h3-rs](https://github.com/jeromefroe/h3-rs))

## Quick Overview

### Conventions / Considerations

* The resolution for H3 indices to be used as Sorted Set score values will always be the highest
  (15). This ensures correct determination of corresponding cells at all resolutions, and also
  provides best results for distance calculations, etc.
* Since elements' scores will be highest resolution H3 indices, their locations will be accurate to
  within 1m squared.
* The terms element and member and interchangeable, as are H3 key and H3 index

### Commands

The following table shows the H3 commands with comparable current commands (if they exist), description and implementation status:

| Impl | H3 Command    | Comp Command | Description |
| :--- | :------------ | :------------ | :---------- |
| Yes  | `H3.STATUS` | - | get status of H3 module (can be used to determine if the module is loaded) |
| Yes  | `H3.ADD key lng1 lat1 elem1 ... [lngN latN elemN]` | `GEOADD` | add elements for H3 indices calculated from given lng/lat values |
| Yes  | `H3.ADDBYINDEX key h3idx1 elem1 ... [h3idxN elemN]` | - | add entries by H3 index instead of lng/lat position |
| Yes  | `H3.INDEX key elem1 ... [elemN]` | `GEOHASH` | return the H3 index for each of the given elements |
| Yes  | `H3.DIST key elem1 elem2 [m\|km\|ft\|mi]` | `GEODIST` | return the distance between two members (centroid to centroid) |
| Yes  | `H3.POS key elem1 ... [elemN]` | `GEOPOS` | return the centroid lng/lat for the given elements |
| Yes  | `H3.SCAN key cursor` | `ZSCAN` | iterate over elements with their H3 indices |
| Yes  | `H3.REMBYINDEX key h3idx1 ... [h3idxN]` | - | remove the elements matching any of the given H3 indices |
| Yes  | `H3.COUNT key h3idx` | `ZCOUNT` | get count of elements contained in the cell of the given H3 index (any resolution is allowed for H3 indices for this command) |
| Yes  | `H3.CELL key h3idx [LIMIT offset count] [WITHINDICES]` | `ZRANGE` | get list of elements contained in the cell of the given H3 index (any resolution is allowed for H3 indices for this command) |
| No   | `H3.RADIUS key lng1 lat1 radius m\|km\|ft\|mi ...` | `GEORADIUS` | return the elements that are within the borders of the area specified by the center location and the maximum distance from the center (the radius) |
| No   | `H3.RADIUSBYINDEX key h3idx1 radius m\|km\|ft\|mi ...` | `GEORADIUSBYMEMBER` | return the elements that are within the border of the area specified by the element's position and the max distance from the position (radius) |
| No   | `H3.SEARCH key [FROMMEMBER elem] [FROMLONLAT lng lat] ...` | `GEOSEARCH` | get list of elements contained in a radius or box |
| No   | `H3.SEARCHSTORE dest source [FROMMEMBER elem] [FROMLONLAT lng lat] ...` | `GEOSEARCHSTORE` | like `H3.SEARCH`, but stores the results in `dest` sorted set |
<!-- | No   | `H3.POLY key lng1 lat1 ... [lngN latN]` | - | get list of elements contained in the polygon defined by the given list of lng/lat | -->

Like the Geo commands, the H3 commands are backed by sorted sets. This means that some actions on the set don't require H3 commands and can be done using sorted set ("Z*") commands, e.g. `ZCARD` and `ZREM`. Although any sorted set commands can be used, those that return scores aren't as useful as the H3 commands that return H3 indices, which is why it is better to use a command like `H3.SCAN` than `ZSCAN`, for example.


## Setup

1. [Install Redis](https://redis.io/topics/quickstart) - the site suggests installing from source
   to ensure the latest, but I use Homebrew on my Mac:

   ```sh
   $ brew install redis
   ```

2. If running as a service, stop it - if installed using Homebrew:

   ```sh
   $ brew services list
   ```

   and if running:

   ```sh
   $ brew services stop redis
   ```

3. [Install Rust](https://www.rust-lang.org/tools/install) - `rustup` is the preferred tool to
   install, but other options are available
4. Git clone redis-h3
5. Build redis-h3

   ```sh
   $ cargo build
   ```

## Running

Run Redis server manually, loading RedisH3 module. An example with initial output:

```sh
$ redis-server --loadmodule target/debug/libredish3.dylib
40142:C 14 May 2021 12:01:47.212 # oO0OoO0OoO0Oo Redis is starting oO0OoO0OoO0Oo
40142:C 14 May 2021 12:01:47.212 # Redis version=6.2.3, bits=64, commit=00000000, modified=0, pid=40142, just started
40142:C 14 May 2021 12:01:47.212 # Configuration loaded
40142:M 14 May 2021 12:01:47.213 * Increased maximum number of open files to 10032 (it was originally set to 2560).
40142:M 14 May 2021 12:01:47.213 * monotonic clock: POSIX clock_gettime
                _._                                                  
           _.-``__ ''-._                                             
      _.-``    `.  `_.  ''-._           Redis 6.2.3 (00000000/0) 64 bit
  .-`` .-```.  ```\/    _.,_ ''-._                                  
 (    '      ,       .-`  | `,    )     Running in standalone mode
 |`-._`-...-` __...-.``-._|'` _.-'|     Port: 6379
 |    `-._   `._    /     _.-'    |     PID: 40142
  `-._    `-._  `-./  _.-'    _.-'                                   
 |`-._`-._    `-.__.-'    _.-'_.-'|                                  
 |    `-._`-._        _.-'_.-'    |           https://redis.io       
  `-._    `-._`-.__.-'_.-'    _.-'                                   
 |`-._`-._    `-.__.-'    _.-'_.-'|                                  
 |    `-._`-._        _.-'_.-'    |                                  
  `-._    `-._`-.__.-'_.-'    _.-'                                   
      `-._    `-.__.-'    _.-'                                       
          `-._        _.-'                                           
              `-.__.-'                                               

40142:M 14 May 2021 12:01:47.216 # Server initialized
40142:M 14 May 2021 12:01:47.617 * Module 'h3' loaded from target/debug/libredish3.dylib
40142:M 14 May 2021 12:01:47.618 * Loading RDB produced by version 6.2.3
40142:M 14 May 2021 12:01:47.618 * RDB age 54215 seconds
40142:M 14 May 2021 12:01:47.618 * RDB memory usage when created 0.98 Mb
40142:M 14 May 2021 12:01:47.618 * DB loaded from disk: 0.001 seconds
40142:M 14 May 2021 12:01:47.618 * Ready to accept connections
```

## Example Session

Here's an example `redis-cli` session that compares Geo and H3 commands:

```sh
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
127.0.0.1:6379> H3.ADDBYINDEX H3Sicily 8f1e9a0ec840645 "Palermo-key" 8f3f35c64acb125 "Catania-key"
(integer) 2
127.0.0.1:6379> H3.ADDBYINDEX H3Sicily 644553099062806085 "Palermo-idx" 645126749795692837 "Catania-idx"
(integer) 2
127.0.0.1:6379> ZCARD H3Sicily
(integer) 6
127.0.0.1:6379> ZSCAN H3Sicily 0
1) "0"
2)  1) "Palermo"
    2) "538352348825157"
    3) "Palermo-idx"
    4) "538352348825157"
    5) "Palermo-key"
    6) "538352348825157"
    7) "Catania"
    8) "1112003081711909"
    9) "Catania-idx"
   10) "1112003081711909"
   11) "Catania-key"
   12) "1112003081711909"
127.0.0.1:6379> H3.SCAN H3Sicily 0
1) "0"
2)  1) "Palermo"
    2) "8f1e9a0ec840645"
    3) "Palermo-idx"
    4) "8f1e9a0ec840645"
    5) "Palermo-key"
    6) "8f1e9a0ec840645"
    7) "Catania"
    8) "8f3f35c64acb125"
    9) "Catania-idx"
   10) "8f3f35c64acb125"
   11) "Catania-key"
   12) "8f3f35c64acb125"
127.0.0.1:6379> H3.SCAN H3Sicily 0 MATCH P*
1) "0"
2) 1) "Palermo"
   2) "8f1e9a0ec840645"
   3) "Palermo-idx"
   4) "8f1e9a0ec840645"
   5) "Palermo-key"
   6) "8f1e9a0ec840645"
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
127.0.0.1:6379> H3.COUNT H3Sicily 833f35fffffffff
(integer) 3
127.0.0.1:6379> H3.CELL H3Sicily 833f35fffffffff
1) "Catania"
2) "Catania-idx"
3) "Catania-key"
127.0.0.1:6379> H3.CELL H3Sicily 833f35fffffffff WITHINDICES
1) "Catania"
2) "8f3f35c64acb125"
3) "Catania-idx"
4) "8f3f35c64acb125"
5) "Catania-key"
6) "8f3f35c64acb125"
127.0.0.1:6379> H3.CELL H3Sicily 833f35fffffffff WITHINDICES LIMIT 0 1
1) "Catania"
2) "8f3f35c64acb125"
127.0.0.1:6379> GEODIST GEOSicily "Catania" "Palermo"
"166274.1516"
127.0.0.1:6379> H3.DIST H3Sicily "Catania" "Palermo"
"166274.6888"
127.0.0.1:6379> GEODIST GEOSicily "Catania" "Palermo" km
"166.2742"
127.0.0.1:6379> H3.DIST H3Sicily "Catania" "Palermo" km
"166.2747"
127.0.0.1:6379> GEODIST GEOSicily "Catania" "Palermo" mi
"103.3182"
127.0.0.1:6379> H3.DIST H3Sicily "Catania" "Palermo" mi
"103.3186"
127.0.0.1:6379> H3.REMBYINDEX H3Sicily 8f3f35c64acb125
(integer) 3
127.0.0.1:6379> H3.SCAN H3Sicily 0
1) "0"
2) 1) "Palermo"
   2) "8f1e9a0ec840645"
   3) "Palermo-idx"
   4) "8f1e9a0ec840645"
   5) "Palermo-key"
   6) "8f1e9a0ec840645"
```

To verify the values, here are some H3 CLI commands with output to compare against:

```sh
$ h3ToGeo -i 8f1e9a0ec840645
38.1155526323 13.3613848732
$ h3ToGeo -i 8f3f35c64acb125
37.5026658629 15.0872703058
$ geoToH3 --lat 38.115556 --lon 13.361389 -r 15
8f1e9a0ec840645
$ geoToH3 --lat 37.502669 --lon 15.087269 -r 15
8f3f35c64acb125
```
