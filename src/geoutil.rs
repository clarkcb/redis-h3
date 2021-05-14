// Some various Geo-related functions

// These are from redis/src/geohash_helper.c
// TODO: use from redis source

// pub const D_R: f64 = std::f64::consts::PI / 180.0;
// pub const R_MAJOR: f64 = 6378137.0;
// pub const R_MINOR: f64 = 6356752.3142;
// pub const RATIO: f64 = R_MINOR / R_MAJOR;
// pub const ECCENT: f64 = (1.0 - (RATIO *RATIO)).sqrt();
// pub const COM: f64 = 0.5 * ECCENT;

pub const DEG_TO_RAD: f64 = 0.017453292519943295769236907684886;
pub const EARTH_RADIUS_IN_METERS: f64 = 6372797.560856;

// pub const MERCATOR_MAX: f64 = 20037726.37;
// pub const MERCATOR_MIN: f64 = -20037726.37;


pub fn deg_rad(ang: f64) -> f64 {
    ang * DEG_TO_RAD
}

pub fn rad_deg(ang: f64) -> f64 {
    ang / DEG_TO_RAD
}

/* Calculate distance using haversin great circle distance formula. */
pub fn geohash_get_distance(lon1d: f64, lat1d: f64, lon2d: f64, lat2d: f64) -> f64 {
    let lat1r: f64 = deg_rad(lat1d);
    let lon1r: f64 = deg_rad(lon1d);
    let lat2r: f64 = deg_rad(lat2d);
    let lon2r: f64 = deg_rad(lon2d);
    let u: f64 = ((lat2r - lat1r) / 2.0).sin();
    let v: f64 = ((lon2r - lon1r) / 2.0).sin();
    return 2.0 * EARTH_RADIUS_IN_METERS *
        (u * u + lat1r.cos() * lat2r.cos() * v * v).sqrt().asin();
}
