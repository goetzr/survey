use anyhow;
use geo::{algorithm::geodesic_destination::GeodesicDestination, Point};

fn main() -> anyhow::Result<()> {
    // -84:9:6.3504, 39:36:12.528
    let start = Point::new(-84.151764, 39.603480);
    // -84:9:12.3372, 39:36:12.474
    //let dest = Point::new(-84.153427, 39.603465);
    // Distance = 142.5 m @ 269:19:45 = 269.3291666667
    let dist = 142.5;
    let az = 269.3291666667;
    let dest = start.geodesic_destination(az, dist);
    println!("Destination lat={}, lon={}", dest.y(), dest.x());

    Ok(())
}
