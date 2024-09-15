use anyhow::{bail, ensure, Context};
use clap::{builder::PathBufValueParser, Arg, Command};
use itertools::Itertools;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tracing::{error, info, trace};

fn main() {
    tracing_subscriber::fmt().with_env_filter("trace").init();

    if let Err(e) = run() {
        error!("{:#}", e);
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = parse_cmdline();
    ensure!(
        args.data_dir.exists(),
        "specified data directory '{}' does not exist",
        args.data_dir.display()
    );

    let mut parcel_bounds = Vec::new();
    for parcel_num in 1..=2 {
        let start = get_starting_location(args.data_dir.as_path(), parcel_num)
            .with_context(|| format!("parcel {parcel_num}: failed to get the starting location"))?;
        let az_dist = get_azimuth_distance(&args.data_dir, parcel_num).with_context(|| {
            format!("parcel {parcel_num}: failed to get the list of azimuth/distance pairs")
        })?;
        let bounds = calc_bounds(start, az_dist)
            .with_context(|| format!("parcel {parcel_num}: failed to calculate the boundaries"))?;
        write_parcel_points_kml(parcel_num, &bounds)
            .with_context(|| "parcel {parcel_num}: failed to write parcel survey points KML")?;
        parcel_bounds.push(bounds);
    }

    write_survey_outline_kml(&parcel_bounds)
        .with_context(|| "failed to write the survey outline KML")?;

    Ok(())
}

struct CmdlineArgs {
    data_dir: PathBuf,
}

fn parse_cmdline() -> CmdlineArgs {
    let cmd = Command::new("create_kml_files")
        .author("Russ Goetz, russgoetz@gmail.com")
        .version("1.0.0")
        .about("Generates a single survey outline KML file containing both parcels and a survey points KML file for each parcel.")
        .arg(
            Arg::new("data_dir")
                .long("data-dir")
                .required(true)
                .value_name("DATA-DIR")
                .value_parser(PathBufValueParser::new())
                .help("The full path to the directory containing the start and azimuth/distance data files for each parcel.")
        );

    let m = cmd.get_matches();
    CmdlineArgs {
        data_dir: m.get_one::<PathBuf>("data_dir").unwrap().clone(),
    }
}

#[derive(Clone)]
struct NamedPoint {
    point: geo::Point,
    name: String,
}

impl NamedPoint {
    fn new(point: geo::Point, name: String) -> Self {
        Self { point, name }
    }

    fn x(&self) -> f64 {
        self.point.x()
    }

    fn y(&self) -> f64 {
        self.point.y()
    }
}

fn split_whitespace_n(data: &str, times: usize) -> SplitWhitespaceN {
    SplitWhitespaceN::new(data, times)
}

struct SplitWhitespaceN<'a> {
    remaining: &'a str,
    times: usize,
}

impl<'a> SplitWhitespaceN<'a> {
    fn new(data: &'a str, times: usize) -> SplitWhitespaceN<'a> {
        let start = data.chars().take_while(|c| c.is_ascii_whitespace()).count();
        Self {
            remaining: &data[start..],
            times,
        }
    }
}

impl<'a> Iterator for SplitWhitespaceN<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.times > 0 {
            self.times -= 1;
            if self.times == 0 {
                Some(self.remaining)
            } else {
                let end = self
                    .remaining
                    .chars()
                    .take_while(|c| !c.is_ascii_whitespace())
                    .count();
                let next = &self.remaining[..end];
                self.remaining = &self.remaining[end..];
                let new_start = self
                    .remaining
                    .chars()
                    .take_while(|c| c.is_ascii_whitespace())
                    .count();
                self.remaining = &self.remaining[new_start..];
                Some(next)
            }
        } else {
            None
        }
    }
}

fn get_starting_location(data_dir: &Path, parcel_num: i32) -> anyhow::Result<NamedPoint> {
    let filename = String::from("parcel") + parcel_num.to_string().as_str() + "_start_lat_lon.txt";
    let mut path = data_dir.to_path_buf();
    path.push(filename);

    let start = fs::read_to_string(&path)
        .context(format!("failed to read '{}'", path.display().to_string()))?;
    let start = start.trim();

    let Some((lat, lon, name)) = split_whitespace_n(&start, 3).collect_tuple() else {
        bail!("failed to split starting location line into parts");
    };

    let lat = lat
        .parse::<f64>()
        .with_context(|| "failed to parse latitude")?;
    let lon = lon
        .parse::<f64>()
        .with_context(|| "failed to parse longitude")?;

    let point = NamedPoint::new(geo::Point::new(lon, lat), name.to_string());
    trace!(
        "parcel {parcel_num}: starting point named {} at ({}, {})",
        point.name,
        point.y(),
        point.x()
    );

    Ok(point)
}

enum FaceDir {
    N,
    S,
}

impl FromStr for FaceDir {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "N" => Ok(FaceDir::N),
            "S" => Ok(FaceDir::S),
            _ => bail!("invalid face direction"),
        }
    }
}

impl fmt::Display for FaceDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dir_str = match self {
            FaceDir::N => "N",
            FaceDir::S => "S",
        };
        f.write_str(dir_str)
    }
}

enum TurnDir {
    E,
    W,
}

impl FromStr for TurnDir {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "E" => Ok(TurnDir::E),
            "W" => Ok(TurnDir::W),
            _ => bail!("invalid turn direction"),
        }
    }
}

impl fmt::Display for TurnDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dir_str = match self {
            TurnDir::E => "E",
            TurnDir::W => "W",
        };
        f.write_str(dir_str)
    }
}

fn get_azimuth_distance(
    data_dir: &Path,
    parcel_num: i32,
) -> anyhow::Result<Vec<(f64, f64, String)>> {
    let filename =
        String::from("parcel") + parcel_num.to_string().as_str() + "_bearing_distance.txt";
    let mut path = data_dir.to_path_buf();
    path.push(filename);

    let file =
        File::open(&path).context(format!("failed to open '{}'", path.display().to_string()))?;
    let reader = BufReader::new(file);

    let mut az_dist = Vec::new();
    for line in reader.lines() {
        // Example line: S 78 03 13 E 171.48 Corner 18
        let line = line.context(format!("failed to read '{}'", path.display().to_string()))?;
        let line = line.trim();

        let Some((face, deg, min, sec, turn, dist_ft, name)) =
            split_whitespace_n(line, 7).collect_tuple()
        else {
            bail!("failed to split bearing/distance line into parts");
        };

        let face = face
            .parse::<FaceDir>()
            .context(format!("invalid face direction '{}'", face))?;
        let deg: f64 = deg
            .parse::<f64>()
            .context(format!("invalid degrees '{}'", deg))?;
        let min: f64 = min
            .parse::<f64>()
            .context(format!("invalid minutes '{}'", min))?;
        let sec = sec
            .parse::<f64>()
            .context(format!("invalid seconds '{}'", sec))?;
        let turn = turn
            .parse::<TurnDir>()
            .context(format!("invalid turn direction '{}'", turn))?;
        let dist_ft: f64 = dist_ft
            .parse::<f64>()
            .context(format!("invalid distance '{}'", dist_ft))?;
        trace!("{face} {deg}° {min}′ {sec}″ {turn}, distance = {dist_ft} ft, name = {name}");

        // Convert bearing as <face> <D:M:S> <turn> to azimuth in degrees decimal.
        let az = bearing_to_azimuth(face, deg, min, sec, turn);
        // Convert distance from feet to meters.
        let dist = dist_ft * 0.3048;
        trace!("\taz = {az}°, dist = {dist} m");

        az_dist.push((az, dist, name.to_string()));
    }

    Ok(az_dist)
}

fn bearing_to_azimuth(face: FaceDir, deg: f64, min: f64, sec: f64, turn: TurnDir) -> f64 {
    let angle = deg + min / 60.0 + sec / 3600.0;

    let mut az = match (face, turn) {
        (FaceDir::N, TurnDir::E) => 0.0 + angle,
        (FaceDir::N, TurnDir::W) => 0.0 - angle,
        (FaceDir::S, TurnDir::E) => 180.0 - angle,
        (FaceDir::S, TurnDir::W) => 180.0 + angle,
    };

    if az < 0.0 {
        az += 360.0;
    }

    az
}

fn calc_bounds(
    start: NamedPoint,
    az_dist: Vec<(f64, f64, String)>,
) -> anyhow::Result<Vec<NamedPoint>> {
    use geo::algorithm::geodesic_destination::GeodesicDestination;

    let mut bounds = vec![start];
    for (idx, (az, dist, name)) in az_dist.into_iter().enumerate() {
        let point = bounds[idx].point.geodesic_destination(az, dist);
        let named_point = NamedPoint::new(point, name);
        bounds.push(named_point);
    }
    let first = bounds.first().unwrap();
    let last = bounds.last().unwrap();
    ensure!(
        (last.x() - first.x()).abs() < 0.000001,
        "x coordinate of last point doesn't match the first point"
    );
    ensure!(
        (last.y() - first.y()).abs() < 0.000001,
        "y coordinate of last point doesn't match the first point"
    );
    // The last point is effectively a copy of the first point, so it can safely be removed.
    bounds.pop();

    trace!("{} boundary points", bounds.len());
    for bound in &bounds {
        trace!(
            "lat = {}, lon = {}, name = {}",
            bound.y(),
            bound.x(),
            bound.name
        );
    }

    Ok(bounds)
}

fn write_survey_outline_kml(parcel_bounds: &Vec<Vec<NamedPoint>>) -> anyhow::Result<()> {
    use std::io::Write;

    let file = File::create("survey_outline.kml")?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "{}", get_leading_kml("Survey Outline")?)?;

    for (idx, bounds) in parcel_bounds.iter().enumerate() {
        writeln!(writer, "\t<Placemark>")?;
        let parcel_name = format!("<name>Parcel {}</name>", idx + 1);
        writeln!(writer, "\t\t{}", parcel_name)?;
        writeln!(writer, "\t\t<styleUrl>#icon-1739-0288D1-nodesc</styleUrl>")?;
        writeln!(writer, "\t\t<Polygon>")?;
        writeln!(writer, "\t\t\t<outerBoundaryIs>")?;
        writeln!(writer, "\t\t\t\t<LinearRing>")?;
        writeln!(writer, "\t\t\t\t\t<coordinates>")?;

        let coords = bounds
            .iter()
            .map(|b| format!("{},{}", b.x(), b.y()))
            .collect::<Vec<String>>()
            .join("\n\t\t\t\t\t\t");
        writeln!(writer, "\t\t\t\t\t\t{coords}")?;

        writeln!(writer, "\t\t\t\t\t</coordinates>")?;
        writeln!(writer, "\t\t\t\t</LinearRing>")?;
        writeln!(writer, "\t\t\t</outerBoundaryIs>")?;
        writeln!(writer, "\t\t</Polygon>")?;
        writeln!(writer, "\t</Placemark>")?;
    }

    writeln!(writer, "{}", get_trailing_kml()?)?;

    Ok(())
}

fn write_parcel_points_kml(parcel_num: i32, bounds: &Vec<NamedPoint>) -> anyhow::Result<()> {
    use std::io::Write;

    let file = File::create(format!("parcel{}_survey_points.kml", parcel_num))?;
    let mut writer = BufWriter::new(file);

    writeln!(
        writer,
        "{}",
        get_leading_kml(format!("Parcel {} Survey Points", parcel_num).as_str())?
    )?;

    for bound in bounds.into_iter() {
        writeln!(writer, "\t<Placemark>")?;
        writeln!(writer, "\t\t<name>{}</name>", bound.name)?;
        writeln!(writer, "\t\t<styleUrl>#icon-1739-0288D1-nodesc</styleUrl>")?;
        writeln!(writer, "\t\t<Point>")?;
        writeln!(writer, "\t\t\t<coordinates>")?;
        writeln!(writer, "{}", format!("\t\t\t\t{},{}", bound.x(), bound.y()))?;
        writeln!(writer, "\t\t\t</coordinates>")?;
        writeln!(writer, "\t\t</Point>")?;
        writeln!(writer, "\t</Placemark>")?;
    }

    writeln!(writer, "{}", get_trailing_kml()?)?;

    Ok(())
}

fn get_leading_kml(name: &str) -> anyhow::Result<String> {
    use std::io::{Cursor, Write};

    let mut writer = Cursor::new(Vec::new());
    writeln!(writer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        writer,
        r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2" xmlns:kml="http://www.opengis.net/kml/2.2" xmlns:atom="http://www.w3.org/2005/Atom">"#
    )?;
    writeln!(writer, "<Document>")?;
    writeln!(writer, "\t<name>{}</name>", name)?;
    writeln!(writer, "{}", get_style_kml())?;

    let kml = String::from_utf8(writer.get_ref().clone())
        .with_context(|| "invalid UTF-8 in leading KML")?;
    Ok(kml)
}

fn get_trailing_kml() -> anyhow::Result<String> {
    use std::io::{Cursor, Write};

    let mut writer = Cursor::new(Vec::new());
    writeln!(writer, "</Document>")?;
    writeln!(writer, "</kml>")?;

    let kml = String::from_utf8(writer.get_ref().clone())
        .with_context(|| "invalid UTF-8 in trailing KML")?;
    Ok(kml)
}

fn get_style_kml() -> String {
    // Color is ABGR, 00 = clear
    r#"
    <StyleMap id="icon-1739-0288D1-nodesc">
        <Pair>
			<key>normal</key>
			<styleUrl>#icon-1739-0288D1-nodesc-normal</styleUrl>
		</Pair>
		<Pair>
			<key>highlight</key>
			<styleUrl>#icon-1739-0288D1-nodesc-highlight</styleUrl>
		</Pair>
	</StyleMap>
	<Style id="icon-1739-0288D1-nodesc-normal">
        <IconStyle>
            <color>ffd18802</color>
            <scale>1</scale>
            <Icon>
                <href>https://www.gstatic.com/mapspro/images/stock/503-wht-blank_maps.png</href>
            </Icon>
        </IconStyle>
        <LabelStyle>
            <scale>0</scale>
        </LabelStyle>
        <BalloonStyle>
            <text><![CDATA[<h3>$[name]</h3>]]></text>
        </BalloonStyle>
		<LineStyle>
			<color>ff0000ff</color>
			<width>3</width>
		</LineStyle>
		<PolyStyle>
            <outline>1</outline>
			<fill>1</fill>
            <color>200000ff</color>
		</PolyStyle>
	</Style>
	<Style id="icon-1739-0288D1-nodesc-highlight">
        <IconStyle>
            <color>ffd18802</color>
            <scale>1</scale>
            <Icon>
                <href>https://www.gstatic.com/mapspro/images/stock/503-wht-blank_maps.png</href>
            </Icon>
        </IconStyle>
        <LabelStyle>
            <scale>1</scale>
        </LabelStyle>
        <BalloonStyle>
            <text><![CDATA[<h3>$[name]</h3>]]></text>
        </BalloonStyle>
		<LineStyle>
			<color>ff0000ff</color>
			<width>3</width>
		</LineStyle>
		<PolyStyle>
            <outline>1</outline>
			<fill>1</fill>
            <color>200000ff</color>
		</PolyStyle>
	</Style>"#
        .to_string()
}

#[cfg(test)]
mod test {
    use super::*;
    use itertools::Itertools;

    #[test]
    fn split_whitespace() {
        let data = "how now brown cow";
        let parts = split_whitespace_n(&data, 3).collect::<Vec<_>>();
        assert_eq!(parts[0], "how");
        assert_eq!(parts[1], "now");
        assert_eq!(parts[2], "brown cow");
    }

    #[test]
    fn split_whitespace_tuple() {
        let data = "how now brown cow";
        let Some((p1, p2, p3)) = split_whitespace_n(&data, 3).collect_tuple() else {
            panic!("failed to collect tuple");
        };
        assert_eq!(p1, "how");
        assert_eq!(p2, "now");
        assert_eq!(p3, "brown cow");
    }
}
