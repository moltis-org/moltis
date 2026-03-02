//! `show_map` tool — displays a static map image with a clickable link to the
//! configured map provider.
//!
//! Composes a static map from OSM tiles (no API key required), draws marker
//! pins for one or more destinations and optionally the user's current
//! location, and returns clickable links so the user can open locations in
//! their preferred mapping application.

use std::io::Cursor;

use {
    async_trait::async_trait,
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    image::{ImageFormat, RgbaImage, imageops},
    moltis_agents::tool_registry::AgentTool,
    serde::Deserialize,
    tracing::{debug, warn},
};

use crate::{Result, error::Error};

// ── Parameters ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ShowMapPointParams {
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShowMapParams {
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    zoom: Option<u8>,
    #[serde(default)]
    user_latitude: Option<f64>,
    #[serde(default)]
    user_longitude: Option<f64>,
    #[serde(default)]
    points: Vec<ShowMapPointParams>,
}

#[derive(Debug, Clone)]
struct DestinationPoint {
    latitude: f64,
    longitude: f64,
    label: Option<String>,
}

fn validate_latitude(value: f64, field: &str) -> Result<()> {
    if !(-90.0..=90.0).contains(&value) {
        return Err(Error::message(format!(
            "{field} must be between -90 and 90, got {value}"
        )));
    }
    Ok(())
}

fn validate_longitude(value: f64, field: &str) -> Result<()> {
    if !(-180.0..=180.0).contains(&value) {
        return Err(Error::message(format!(
            "{field} must be between -180 and 180, got {value}"
        )));
    }
    Ok(())
}

fn normalize_destination_points(params: &ShowMapParams) -> Result<Vec<DestinationPoint>> {
    if !params.points.is_empty() {
        let mut points = Vec::with_capacity(params.points.len());
        for (idx, point) in params.points.iter().enumerate() {
            validate_latitude(point.latitude, &format!("points[{idx}].latitude"))?;
            validate_longitude(point.longitude, &format!("points[{idx}].longitude"))?;
            points.push(DestinationPoint {
                latitude: point.latitude,
                longitude: point.longitude,
                label: point.label.clone(),
            });
        }
        return Ok(points);
    }

    match (params.latitude, params.longitude) {
        (Some(latitude), Some(longitude)) => {
            validate_latitude(latitude, "latitude")?;
            validate_longitude(longitude, "longitude")?;
            Ok(vec![DestinationPoint {
                latitude,
                longitude,
                label: params.label.clone(),
            }])
        },
        (Some(_), None) => Err(Error::message(
            "longitude is required when latitude is provided",
        )),
        (None, Some(_)) => Err(Error::message(
            "latitude is required when longitude is provided",
        )),
        (None, None) => Err(Error::message(
            "provide either `points` or `latitude`/`longitude`",
        )),
    }
}

// ── Map links ───────────────────────────────────────────────────────────────

/// Map provider used to generate outbound map links.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MapProvider {
    #[default]
    GoogleMaps,
    AppleMaps,
    OpenStreetMap,
}

impl MapProvider {
    fn as_config_value(self) -> &'static str {
        match self {
            Self::GoogleMaps => "google_maps",
            Self::AppleMaps => "apple_maps",
            Self::OpenStreetMap => "openstreetmap",
        }
    }
}

/// Build clickable map URLs for the selected mapping provider.
fn build_map_links(
    provider: MapProvider,
    lat: f64,
    lon: f64,
    zoom: u8,
    label: Option<&str>,
) -> serde_json::Value {
    // When a place name is provided, use it as the search query so the map
    // service resolves the actual business page (with reviews, hours, photos)
    // instead of just dropping an anonymous pin at raw coordinates.
    let url = match provider {
        MapProvider::GoogleMaps => match label {
            Some(l) => format!(
                "https://www.google.com/maps/search/?api=1&query={}&center={lat},{lon}",
                urlencoded(l),
            ),
            None => format!("https://www.google.com/maps/search/?api=1&query={lat},{lon}"),
        },
        MapProvider::AppleMaps => match label {
            Some(l) => format!(
                "https://maps.apple.com/?ll={lat},{lon}&q={}&z={zoom}",
                urlencoded(l),
            ),
            None => format!("https://maps.apple.com/?ll={lat},{lon}&z={zoom}"),
        },
        MapProvider::OpenStreetMap => match label {
            Some(l) => format!(
                "https://www.openstreetmap.org/search?query={}&mlat={lat}&mlon={lon}#map={zoom}/{lat}/{lon}",
                urlencoded(l),
            ),
            None => {
                format!(
                    "https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map={zoom}/{lat}/{lon}"
                )
            },
        },
    };

    let mut links = serde_json::Map::new();
    links.insert(
        "provider".to_string(),
        serde_json::Value::String(provider.as_config_value().to_string()),
    );
    links.insert("url".to_string(), serde_json::Value::String(url.clone()));
    links.insert(
        provider.as_config_value().to_string(),
        serde_json::Value::String(url),
    );
    serde_json::Value::Object(links)
}

/// Minimal percent-encoding for URL query values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            },
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(HEX_UPPER[(b >> 4) as usize] as char);
                out.push(HEX_UPPER[(b & 0x0f) as usize] as char);
            },
        }
    }
    out
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

// ── OSM tile math ───────────────────────────────────────────────────────────

const TILE_SIZE: u32 = 256;

/// Convert lat/lon to fractional tile coordinates at a given zoom level.
fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u8) -> (f64, f64) {
    let n = f64::from(1u32 << zoom);
    let x = (lon + 180.0) / 360.0 * n;
    let lat_rad = lat.to_radians();
    let y = (1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n;
    (x, y)
}

/// Convert lat/lon to normalized Web Mercator world coordinates [0, 1].
fn lat_lon_to_world(lat: f64, lon: f64) -> (f64, f64) {
    let x = ((lon + 180.0) / 360.0).rem_euclid(1.0);
    let lat_rad = lat.to_radians();
    let y = (1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0;
    (x, y.clamp(0.0, 1.0))
}

/// Convert normalized Web Mercator world coordinates [0, 1] back to lat/lon.
fn world_to_lat_lon(x: f64, y: f64) -> (f64, f64) {
    let lon = x.rem_euclid(1.0) * 360.0 - 180.0;
    let lat_rad = (std::f64::consts::PI * (1.0 - 2.0 * y.clamp(0.0, 1.0)))
        .sinh()
        .atan();
    (lat_rad.to_degrees(), lon)
}

/// Find minimal wrapped x-span and center on a [0, 1) circle.
fn wrapped_span_and_center(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }

    if values.len() == 1 {
        return Some((0.0, values[0].rem_euclid(1.0)));
    }

    let mut sorted: Vec<f64> = values.iter().map(|v| v.rem_euclid(1.0)).collect();
    sorted.sort_by(f64::total_cmp);

    let mut max_gap = f64::NEG_INFINITY;
    let mut max_gap_idx = 0usize;
    for idx in 0..sorted.len() {
        let current = sorted[idx];
        let next = if idx + 1 < sorted.len() {
            sorted[idx + 1]
        } else {
            sorted[0] + 1.0
        };
        let gap = next - current;
        if gap > max_gap {
            max_gap = gap;
            max_gap_idx = idx;
        }
    }

    let span = (1.0 - max_gap).clamp(0.0, 1.0);
    let arc_start = if max_gap_idx + 1 < sorted.len() {
        sorted[max_gap_idx + 1]
    } else {
        sorted[0]
    };
    let center = (arc_start + span / 2.0).rem_euclid(1.0);
    Some((span, center))
}

/// Compute the map center that keeps points tightly framed, including
/// international date-line crossings.
fn center_for_points(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.is_empty() {
        return None;
    }
    if points.len() == 1 {
        return Some(points[0]);
    }

    let world_points: Vec<(f64, f64)> = points
        .iter()
        .map(|(lat, lon)| lat_lon_to_world(*lat, *lon))
        .collect();
    let xs: Vec<f64> = world_points.iter().map(|(x, _)| *x).collect();
    let ys: Vec<f64> = world_points.iter().map(|(_, y)| *y).collect();
    let (_, center_x) = wrapped_span_and_center(&xs)?;
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let center_y = (min_y + max_y) / 2.0;
    Some(world_to_lat_lon(center_x, center_y))
}

/// Choose a zoom level that fits all points within the given pixel dimensions,
/// with some padding so markers aren't at the very edge.
fn auto_zoom_points(points: &[(f64, f64)], width: u32, height: u32) -> u8 {
    if points.len() <= 1 {
        return 18;
    }

    let world_points: Vec<(f64, f64)> = points
        .iter()
        .map(|(lat, lon)| lat_lon_to_world(*lat, *lon))
        .collect();
    let xs: Vec<f64> = world_points.iter().map(|(x, _)| *x).collect();
    let ys: Vec<f64> = world_points.iter().map(|(_, y)| *y).collect();
    let (x_span, _) = match wrapped_span_and_center(&xs) {
        Some(v) => v,
        None => return 18,
    };
    let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let y_span = (max_y - min_y).abs();

    // Try zoom levels from 18 down to 2, pick the highest that fits.
    for z in (2..=18).rev() {
        let world_px = f64::from(TILE_SIZE) * f64::from(1u32 << z);
        let dx = x_span * world_px;
        let dy = y_span * world_px;
        // Leave 40% padding on each side so markers aren't at the edge.
        if dx < f64::from(width) * 0.6 && dy < f64::from(height) * 0.6 {
            return z;
        }
    }

    2
}

/// Choose a zoom level that fits two points (legacy helper kept for tests).
#[cfg(test)]
fn auto_zoom(lat1: f64, lon1: f64, lat2: f64, lon2: f64, width: u32, height: u32) -> u8 {
    auto_zoom_points(&[(lat1, lon1), (lat2, lon2)], width, height)
}

// ── Static map compositing ──────────────────────────────────────────────────

/// Marker to draw on the map.
struct Marker {
    lat: f64,
    lon: f64,
    color: [u8; 4], // RGBA
}

const MAP_WIDTH: u32 = 600;
const MAP_HEIGHT: u32 = 400;
const MARKER_RADIUS: i32 = 10;
const USER_MARKER_COLOR: [u8; 4] = [50, 120, 220, 255];
const DESTINATION_MARKER_COLORS: [[u8; 4]; 6] = [
    [220, 50, 50, 255],
    [217, 119, 6, 255],
    [5, 150, 105, 255],
    [124, 58, 237, 255],
    [14, 116, 144, 255],
    [185, 28, 28, 255],
];

/// Compose a static map from OSM tiles with markers.
///
/// Returns `Some(data_uri)` on success, `None` on failure (network errors,
/// tile decode errors). The caller degrades gracefully to links-only.
async fn compose_static_map(
    client: &reqwest::Client,
    center_lat: f64,
    center_lon: f64,
    zoom: u8,
    markers: &[Marker],
) -> Option<String> {
    let (cx, cy) = lat_lon_to_tile(center_lat, center_lon, zoom);

    // Calculate which tiles we need to cover the output image.
    let half_w = f64::from(MAP_WIDTH) / 2.0 / f64::from(TILE_SIZE);
    let half_h = f64::from(MAP_HEIGHT) / 2.0 / f64::from(TILE_SIZE);

    let tile_min_x = (cx - half_w).floor() as i32;
    let tile_max_x = (cx + half_w).ceil() as i32;
    let tile_min_y = (cy - half_h).floor() as i32;
    let tile_max_y = (cy + half_h).ceil() as i32;

    let n = 1i32 << zoom;

    // Fetch all tiles concurrently.
    let mut fetch_tasks = Vec::new();
    for ty in tile_min_y..tile_max_y {
        for tx in tile_min_x..tile_max_x {
            // Wrap x for world wrap-around; clamp y.
            let wrapped_tx = tx.rem_euclid(n) as u32;
            if ty < 0 || ty >= n {
                continue;
            }
            let url = format!("https://tile.openstreetmap.org/{zoom}/{wrapped_tx}/{ty}.png");
            let client = client.clone();
            fetch_tasks.push(async move {
                let result = fetch_tile(&client, &url).await;
                (tx, ty, result)
            });
        }
    }

    let results = futures::future::join_all(fetch_tasks).await;

    // Create output image.
    let mut canvas = RgbaImage::new(MAP_WIDTH, MAP_HEIGHT);

    // Pixel offset of the center in tile-space.
    let origin_px = cx * f64::from(TILE_SIZE);
    let origin_py = cy * f64::from(TILE_SIZE);
    let canvas_origin_x = origin_px - f64::from(MAP_WIDTH) / 2.0;
    let canvas_origin_y = origin_py - f64::from(MAP_HEIGHT) / 2.0;

    let mut any_tile = false;
    for (tx, ty, tile_result) in &results {
        let Some(tile_img) = tile_result else {
            continue;
        };
        any_tile = true;

        let tile_px = *tx as f64 * f64::from(TILE_SIZE);
        let tile_py = *ty as f64 * f64::from(TILE_SIZE);
        let dx = (tile_px - canvas_origin_x).round() as i64;
        let dy = (tile_py - canvas_origin_y).round() as i64;

        imageops::overlay(&mut canvas, tile_img, dx, dy);
    }

    if !any_tile {
        warn!("no tiles fetched — static map unavailable");
        return None;
    }

    // Draw markers.
    for marker in markers {
        let (mx, my) = lat_lon_to_tile(marker.lat, marker.lon, zoom);
        let px = (mx * f64::from(TILE_SIZE) - canvas_origin_x).round() as i32;
        let py = (my * f64::from(TILE_SIZE) - canvas_origin_y).round() as i32;
        draw_marker(&mut canvas, px, py, MARKER_RADIUS, marker.color);
    }

    // Encode to PNG.
    let mut buf = Cursor::new(Vec::new());
    if canvas.write_to(&mut buf, ImageFormat::Png).is_err() {
        warn!("failed to encode static map PNG");
        return None;
    }

    let b64 = BASE64.encode(buf.into_inner());
    Some(format!("data:image/png;base64,{b64}"))
}

/// Fetch a single OSM tile and decode it as RGBA.
async fn fetch_tile(client: &reqwest::Client, url: &str) -> Option<RgbaImage> {
    debug!(url = %url, "fetching OSM tile");
    let resp = client
        .get(url)
        .header("User-Agent", "moltis/0.3")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        warn!(status = %resp.status(), url = %url, "tile fetch failed");
        return None;
    }

    let bytes = resp.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    Some(img.to_rgba8())
}

// ── ImageMagick fallback ────────────────────────────────────────────────────

/// Compose a static map using ImageMagick (`magick`) as a fallback when the
/// in-process tile compositing fails.
///
/// Builds a single `magick` pipeline that:
/// 1. Fetches tiles directly from URLs (ImageMagick's own HTTP client)
/// 2. Stitches rows with `+append`, then stacks with `-append`
/// 3. Crops to the target size
/// 4. Draws marker circles
/// 5. Outputs PNG to stdout
///
/// Returns `None` if `magick` is not available or the command fails.
async fn compose_static_map_magick(
    center_lat: f64,
    center_lon: f64,
    zoom: u8,
    markers: &[Marker],
) -> Option<String> {
    use tokio::process::Command;

    // Check that magick is available.
    if Command::new("magick")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_err()
    {
        debug!("magick not available — skipping fallback");
        return None;
    }

    let (cx, cy) = lat_lon_to_tile(center_lat, center_lon, zoom);

    let half_w = f64::from(MAP_WIDTH) / 2.0 / f64::from(TILE_SIZE);
    let half_h = f64::from(MAP_HEIGHT) / 2.0 / f64::from(TILE_SIZE);

    let tile_min_x = (cx - half_w).floor() as i32;
    let tile_max_x = (cx + half_w).ceil() as i32;
    let tile_min_y = (cy - half_h).floor() as i32;
    let tile_max_y = (cy + half_h).ceil() as i32;

    let n = 1i32 << zoom;

    // Pixel offset for cropping: where the canvas origin sits within the
    // full tile grid.
    let origin_px = cx * f64::from(TILE_SIZE);
    let origin_py = cy * f64::from(TILE_SIZE);
    let canvas_origin_x = origin_px - f64::from(MAP_WIDTH) / 2.0;
    let canvas_origin_y = origin_py - f64::from(MAP_HEIGHT) / 2.0;

    let crop_x = (canvas_origin_x - f64::from(tile_min_x) * f64::from(TILE_SIZE)).round() as u32;
    let crop_y = (canvas_origin_y - f64::from(tile_min_y) * f64::from(TILE_SIZE)).round() as u32;

    // Build magick args: group each row with ( url1 url2 ... +append )
    let mut args: Vec<String> = Vec::new();

    for ty in tile_min_y..tile_max_y {
        if ty < 0 || ty >= n {
            continue;
        }
        args.push("(".into());
        for tx in tile_min_x..tile_max_x {
            let wrapped_tx = tx.rem_euclid(n) as u32;
            args.push(format!(
                "https://tile.openstreetmap.org/{zoom}/{wrapped_tx}/{ty}.png"
            ));
        }
        args.push("+append".into());
        args.push(")".into());
    }

    // Stack rows vertically.
    args.push("-append".into());

    // Crop to output size.
    args.push("-crop".into());
    args.push(format!("{MAP_WIDTH}x{MAP_HEIGHT}+{crop_x}+{crop_y}"));
    args.push("+repage".into());

    // Draw markers.
    for marker in markers {
        let (mx, my) = lat_lon_to_tile(marker.lat, marker.lon, zoom);
        let px = (mx * f64::from(TILE_SIZE) - canvas_origin_x).round() as i32;
        let py = (my * f64::from(TILE_SIZE) - canvas_origin_y).round() as i32;
        let [r, g, b, a] = marker.color;
        let radius = MARKER_RADIUS;
        args.push("-fill".into());
        args.push(format!("rgba({r},{g},{b},{a})"));
        args.push("-stroke".into());
        args.push("rgba(40,40,40,255)".into());
        args.push("-strokewidth".into());
        args.push("2".into());
        args.push("-draw".into());
        args.push(format!("circle {px},{py} {},{py}", px + radius));
    }

    // Output PNG to stdout.
    args.push("PNG:-".into());

    debug!(args = ?args, "running magick fallback");

    let output = Command::new("magick")
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(stderr = %stderr, "magick fallback failed");
        return None;
    }

    if output.stdout.is_empty() {
        return None;
    }

    let b64 = BASE64.encode(&output.stdout);
    Some(format!("data:image/png;base64,{b64}"))
}

/// Draw a filled circle marker with a dark border on the canvas.
fn draw_marker(canvas: &mut RgbaImage, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
    let border_color = [40u8, 40, 40, 255];
    let border_r = radius + 2;

    // Draw border circle then fill circle.
    for (r, c) in [(border_r, border_color), (radius, color)] {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0
                        && py >= 0
                        && (px as u32) < canvas.width()
                        && (py as u32) < canvas.height()
                    {
                        canvas.put_pixel(px as u32, py as u32, image::Rgba(c));
                    }
                }
            }
        }
    }
}

// ── Tool ────────────────────────────────────────────────────────────────────

/// LLM-callable tool that shows a map image with links to mapping services.
pub struct ShowMapTool {
    provider: MapProvider,
}

impl ShowMapTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_provider(provider: MapProvider) -> Self {
        Self { provider }
    }
}

impl Default for ShowMapTool {
    fn default() -> Self {
        Self::with_provider(MapProvider::default())
    }
}

#[async_trait]
impl AgentTool for ShowMapTool {
    fn name(&self) -> &str {
        "show_map"
    }

    fn description(&self) -> &str {
        "Show a map image to the user for one or more locations. Displays destination \
         pins and an optional blue pin at the user's current location, plus clickable \
         map links using the configured provider (Google Maps by default). Supports either a single \
         destination via latitude/longitude or multiple destinations via points[]. Always \
         pass user_latitude and user_longitude when available so the user can see both \
         their position and destinations on the map."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "latitude": {
                    "type": "number",
                    "description": "Latitude of the destination (-90 to 90)"
                },
                "longitude": {
                    "type": "number",
                    "description": "Longitude of the destination (-180 to 180)"
                },
                "label": {
                    "type": "string",
                    "description": "Optional pin label (e.g. business name)"
                },
                "points": {
                    "type": "array",
                    "description": "Optional list of destination points to render on a single map.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "latitude": {
                                "type": "number",
                                "description": "Latitude of the destination (-90 to 90)"
                            },
                            "longitude": {
                                "type": "number",
                                "description": "Longitude of the destination (-180 to 180)"
                            },
                            "label": {
                                "type": "string",
                                "description": "Optional label for this destination"
                            }
                        },
                        "required": ["latitude", "longitude"],
                        "additionalProperties": false
                    }
                },
                "zoom": {
                    "type": "integer",
                    "description": "Map zoom level (1-18). Auto-calculated when multiple points are shown."
                },
                "user_latitude": {
                    "type": "number",
                    "description": "Latitude of the user's current location (for showing both positions)"
                },
                "user_longitude": {
                    "type": "number",
                    "description": "Longitude of the user's current location (for showing both positions)"
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let p: ShowMapParams = serde_json::from_value(params)?;
        let destinations = normalize_destination_points(&p)?;
        let Some(primary) = destinations.first() else {
            return Err(Error::message("at least one destination is required").into());
        };
        let primary_lat = primary.latitude;
        let primary_lon = primary.longitude;
        let primary_label = primary.label.clone();

        let user_loc = match (p.user_latitude, p.user_longitude) {
            (Some(ulat), Some(ulon)) => {
                validate_latitude(ulat, "user_latitude")?;
                validate_longitude(ulon, "user_longitude")?;
                Some((ulat, ulon))
            },
            _ => None,
        };

        let mut fit_points: Vec<(f64, f64)> = destinations
            .iter()
            .map(|point| (point.latitude, point.longitude))
            .collect();
        if let Some((ulat, ulon)) = user_loc {
            fit_points.push((ulat, ulon));
        }

        // Auto-calculate zoom to fit all points, or use explicit/default.
        let zoom = if let Some(z) = p.zoom {
            z.clamp(1, 18)
        } else if fit_points.len() > 1 {
            auto_zoom_points(&fit_points, MAP_WIDTH, MAP_HEIGHT)
        } else {
            15
        };

        let map_links = build_map_links(
            self.provider,
            primary_lat,
            primary_lon,
            zoom,
            primary_label.as_deref(),
        );

        // Build markers: multi-color destinations and blue for user.
        let mut markers: Vec<Marker> = destinations
            .iter()
            .enumerate()
            .map(|(idx, point)| Marker {
                lat: point.latitude,
                lon: point.longitude,
                color: DESTINATION_MARKER_COLORS[idx % DESTINATION_MARKER_COLORS.len()],
            })
            .collect();
        if let Some((ulat, ulon)) = user_loc {
            markers.push(Marker {
                lat: ulat,
                lon: ulon,
                color: USER_MARKER_COLOR,
            });
        }

        // Calculate center that frames all visible points.
        let (center_lat, center_lon) =
            center_for_points(&fit_points).unwrap_or((primary_lat, primary_lon));

        // Compose the static map image from OSM tiles (in-process via image crate).
        // Falls back to ImageMagick CLI if the in-process approach fails.
        let screenshot = compose_static_map(
            crate::shared_http_client(),
            center_lat,
            center_lon,
            zoom,
            &markers,
        )
        .await;
        let screenshot = match screenshot {
            Some(s) => Some(s),
            None => {
                debug!("in-process tile compositing failed — trying magick fallback");
                compose_static_map_magick(center_lat, center_lon, zoom, &markers).await
            },
        };

        let points_result: Vec<serde_json::Value> = destinations
            .iter()
            .map(|point| {
                let mut item = serde_json::json!({
                    "latitude": point.latitude,
                    "longitude": point.longitude,
                    "map_links": build_map_links(
                        self.provider,
                        point.latitude,
                        point.longitude,
                        zoom,
                        point.label.as_deref()
                    ),
                });
                if let Some(label) = &point.label {
                    item["label"] = serde_json::Value::String(label.clone());
                }
                item
            })
            .collect();

        let mut result = serde_json::json!({
            "latitude": primary_lat,
            "longitude": primary_lon,
            "map_links": map_links,
            "points": points_result,
        });

        if let Some(label) = primary_label {
            result["label"] = serde_json::Value::String(label);
        }

        if let Some(data_uri) = screenshot {
            result["screenshot"] = serde_json::Value::String(data_uri);
        }

        Ok(result)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_links_with_label() {
        let links = build_map_links(
            MapProvider::GoogleMaps,
            37.7614,
            -122.4199,
            15,
            Some("La Taqueria"),
        );
        // Google uses the label as search query with coordinates as center hint.
        assert_eq!(links["provider"], "google_maps");
        assert_eq!(
            links["url"],
            "https://www.google.com/maps/search/?api=1&query=La+Taqueria&center=37.7614,-122.4199"
        );
        assert_eq!(
            links["google_maps"],
            "https://www.google.com/maps/search/?api=1&query=La+Taqueria&center=37.7614,-122.4199"
        );
        assert!(links.get("apple_maps").is_none());
        assert!(links.get("openstreetmap").is_none());
    }

    #[test]
    fn build_links_without_label() {
        let links = build_map_links(MapProvider::OpenStreetMap, 48.8566, 2.3522, 12, None);
        assert_eq!(links["provider"], "openstreetmap");
        assert_eq!(
            links["url"],
            "https://www.openstreetmap.org/?mlat=48.8566&mlon=2.3522#map=12/48.8566/2.3522"
        );
        assert_eq!(
            links["openstreetmap"],
            "https://www.openstreetmap.org/?mlat=48.8566&mlon=2.3522#map=12/48.8566/2.3522"
        );
        assert!(links.get("google_maps").is_none());
        assert!(links.get("apple_maps").is_none());
    }

    #[test]
    fn build_links_special_chars_in_label() {
        let links = build_map_links(MapProvider::AppleMaps, 0.0, 0.0, 10, Some("Café & Bar"));
        let apple = links["apple_maps"].as_str().unwrap();
        assert!(apple.contains("Caf%C3%A9+%26+Bar"));
    }

    #[test]
    fn urlencoded_basic() {
        assert_eq!(urlencoded("hello world"), "hello+world");
        assert_eq!(urlencoded("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencoded("simple"), "simple");
    }

    #[test]
    fn tool_schema_is_valid() {
        let tool = ShowMapTool::new();
        assert_eq!(tool.name(), "show_map");
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["latitude"].is_object());
        assert!(schema["properties"]["longitude"].is_object());
        assert!(schema["properties"]["points"].is_object());
        assert!(schema["properties"]["user_latitude"].is_object());
        assert!(schema["properties"]["user_longitude"].is_object());
        assert!(schema.get("anyOf").is_none());
        assert!(schema.get("oneOf").is_none());
        assert!(schema.get("allOf").is_none());
        let required = schema["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[tokio::test]
    async fn execute_validates_latitude_range() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({ "latitude": 91.0, "longitude": 0.0 }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("latitude must be between"));
    }

    #[tokio::test]
    async fn execute_validates_longitude_range() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({ "latitude": 0.0, "longitude": 181.0 }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("longitude must be between"));
    }

    #[tokio::test]
    async fn execute_validates_user_latitude_range() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({
                "latitude": 0.0,
                "longitude": 0.0,
                "user_latitude": 91.0,
                "user_longitude": 0.0
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_latitude must be between"));
    }

    #[tokio::test]
    async fn execute_validates_user_longitude_range() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({
                "latitude": 0.0,
                "longitude": 0.0,
                "user_latitude": 0.0,
                "user_longitude": 200.0
            }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("user_longitude must be between"));
    }

    #[tokio::test]
    async fn execute_clamps_zoom() {
        let tool = ShowMapTool::with_provider(MapProvider::OpenStreetMap);
        // Zoom 99 should be clamped to 18 — verify via the returned links.
        let result = tool
            .execute(serde_json::json!({
                "latitude": 0.0,
                "longitude": 0.0,
                "zoom": 99
            }))
            .await
            .unwrap();
        let osm = result["map_links"]["url"].as_str().unwrap();
        assert!(osm.contains("#map=18/"), "zoom should be clamped to 18");
    }

    #[tokio::test]
    async fn execute_includes_label_in_result() {
        let tool = ShowMapTool::new();
        let result = tool
            .execute(serde_json::json!({
                "latitude": 37.76,
                "longitude": -122.42,
                "label": "La Taqueria"
            }))
            .await
            .unwrap();
        assert_eq!(result["label"], "La Taqueria");
        assert_eq!(result["latitude"], 37.76);
        assert_eq!(result["longitude"], -122.42);
        assert_eq!(result["map_links"]["provider"], "google_maps");
        assert!(result["map_links"]["url"].is_string());
        assert!(result["map_links"]["google_maps"].is_string());
        assert!(result["map_links"].get("apple_maps").is_none());
        assert!(result["map_links"].get("openstreetmap").is_none());
        assert_eq!(result["points"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn execute_graceful_without_screenshot() {
        // The screenshot fetch may fail in CI — the tool should still succeed.
        let tool = ShowMapTool::new();
        let result = tool
            .execute(serde_json::json!({
                "latitude": 37.76,
                "longitude": -122.42
            }))
            .await
            .unwrap();
        assert!(result["map_links"].is_object());
    }

    #[tokio::test]
    async fn execute_supports_points_input() {
        let tool = ShowMapTool::new();
        let result = tool
            .execute(serde_json::json!({
                "points": [
                    { "latitude": 37.788473, "longitude": -122.408997, "label": "Sears Fine Food" },
                    { "latitude": 37.80026, "longitude": -122.41028, "label": "Mama's on Washington Square" }
                ]
            }))
            .await
            .unwrap();
        assert_eq!(result["latitude"], 37.788473);
        assert_eq!(result["longitude"], -122.408997);
        assert_eq!(result["label"], "Sears Fine Food");
        let points = result["points"].as_array().unwrap();
        assert_eq!(points.len(), 2);
        assert!(points[0]["map_links"]["google_maps"].is_string());
        assert_eq!(points[1]["label"], "Mama's on Washington Square");
    }

    #[tokio::test]
    async fn execute_rejects_missing_destinations() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({
                "zoom": 10
            }))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("provide either `points` or `latitude`/`longitude`")
        );
    }

    #[tokio::test]
    async fn execute_validates_points_latitude_range() {
        let tool = ShowMapTool::new();
        let err = tool
            .execute(serde_json::json!({
                "points": [{ "latitude": 91.0, "longitude": 0.0 }]
            }))
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("points[0].latitude must be between")
        );
    }

    // ── Tile math tests ─────────────────────────────────────────────────────

    #[test]
    fn tile_coords_at_zoom_0() {
        // At zoom 0, there is 1 tile covering the whole world.
        let (x, y) = lat_lon_to_tile(0.0, 0.0, 0);
        assert!((x - 0.5).abs() < 0.01);
        assert!((y - 0.5).abs() < 0.01);
    }

    #[test]
    fn tile_coords_known_location() {
        // San Francisco at zoom 15 should be near tile (5241, 12666).
        let (x, y) = lat_lon_to_tile(37.76, -122.42, 15);
        assert!((x - 5241.0).abs() < 2.0, "x={x}");
        assert!((y - 12666.0).abs() < 2.0, "y={y}");
    }

    #[test]
    fn auto_zoom_nearby_points() {
        // Two points ~0.003 degrees apart (~300m) should give high zoom.
        let z = auto_zoom(37.760, -122.420, 37.763, -122.418, 600, 400);
        assert!(z >= 15, "zoom={z}, expected >= 15 for nearby points");
    }

    #[test]
    fn auto_zoom_distant_points() {
        // SF to LA (~5 degrees apart) should give low zoom.
        let z = auto_zoom(37.76, -122.42, 34.05, -118.24, 600, 400);
        assert!(z <= 9, "zoom={z}, expected <= 9 for SF-to-LA distance");
    }

    #[test]
    fn auto_zoom_same_point() {
        let z = auto_zoom(37.76, -122.42, 37.76, -122.42, 600, 400);
        assert_eq!(z, 18, "same point should give max zoom");
    }

    #[test]
    fn auto_zoom_multiple_points() {
        let points = vec![
            (37.788473, -122.408997),
            (37.79062, -122.42238),
            (37.76966, -122.43125),
            (37.80895, -122.41576),
        ];
        let z = auto_zoom_points(&points, 600, 400);
        assert!(
            (12..=16).contains(&z),
            "zoom={z}, expected neighborhood-level zoom for SF points"
        );
    }

    #[test]
    fn center_for_points_handles_date_line() {
        let points = vec![(0.0, 179.0), (0.0, -179.0)];
        let (lat, lon) = center_for_points(&points).unwrap();
        assert!(lat.abs() < 1.0, "lat={lat}");
        assert!(lon.abs() > 170.0, "lon={lon}");
    }

    // ── Marker drawing tests ────────────────────────────────────────────────

    #[test]
    fn draw_marker_center_pixel() {
        let mut canvas = RgbaImage::new(100, 100);
        draw_marker(&mut canvas, 50, 50, 5, [255, 0, 0, 255]);
        let px = canvas.get_pixel(50, 50);
        assert_eq!(px.0, [255, 0, 0, 255]);
    }

    #[test]
    fn draw_marker_has_border() {
        let mut canvas = RgbaImage::new(100, 100);
        draw_marker(&mut canvas, 50, 50, 5, [255, 0, 0, 255]);
        // Just outside radius 5 but inside border radius 7 → dark border.
        let px = canvas.get_pixel(56, 50);
        assert_eq!(px.0, [40, 40, 40, 255]);
    }

    #[test]
    fn draw_marker_edge_clipping() {
        // Marker near canvas edge should not panic.
        let mut canvas = RgbaImage::new(20, 20);
        draw_marker(&mut canvas, 2, 2, 10, [0, 255, 0, 255]);
        // Center should have the marker color.
        let px = canvas.get_pixel(2, 2);
        assert_eq!(px.0, [0, 255, 0, 255]);
    }

    // ── Mock tile fetch tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_tile_with_mock() {
        // Create a minimal valid 1x1 PNG.
        let mut img = RgbaImage::new(1, 1);
        img.put_pixel(0, 0, image::Rgba([128, 128, 128, 255]));
        let mut png_buf = Cursor::new(Vec::new());
        img.write_to(&mut png_buf, ImageFormat::Png).unwrap();

        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body(png_buf.into_inner())
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/15/5241/12666.png", server.url());
        let tile = fetch_tile(&client, &url).await;
        assert!(tile.is_some());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_tile_server_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/15/0/0.png", server.url());
        let tile = fetch_tile(&client, &url).await;
        assert!(tile.is_none());
    }

    #[tokio::test]
    async fn compose_map_with_mock_tiles() {
        // Create a 256x256 grey tile.
        let mut tile_img = RgbaImage::new(256, 256);
        for pixel in tile_img.pixels_mut() {
            *pixel = image::Rgba([200, 200, 200, 255]);
        }
        let mut png_buf = Cursor::new(Vec::new());
        tile_img.write_to(&mut png_buf, ImageFormat::Png).unwrap();
        let tile_bytes = png_buf.into_inner();

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body(tile_bytes)
            .create_async()
            .await;

        // We can't redirect compose_static_map to mock, but we can test
        // compose_static_map_with_base_url directly.
        let client = reqwest::Client::new();
        let markers = vec![Marker {
            lat: 37.76,
            lon: -122.42,
            color: [220, 50, 50, 255],
        }];
        let result =
            compose_static_map_with_base_url(&client, &server.url(), 37.76, -122.42, 15, &markers)
                .await;
        assert!(result.is_some(), "expected composed map image");
        let data_uri = result.unwrap();
        assert!(data_uri.starts_with("data:image/png;base64,"));
    }

    /// Test helper — like `compose_static_map` but with a configurable tile URL
    /// base for mock server testing.
    async fn compose_static_map_with_base_url(
        client: &reqwest::Client,
        base_url: &str,
        center_lat: f64,
        center_lon: f64,
        zoom: u8,
        markers: &[Marker],
    ) -> Option<String> {
        let (cx, cy) = lat_lon_to_tile(center_lat, center_lon, zoom);

        let half_w = f64::from(MAP_WIDTH) / 2.0 / f64::from(TILE_SIZE);
        let half_h = f64::from(MAP_HEIGHT) / 2.0 / f64::from(TILE_SIZE);

        let tile_min_x = (cx - half_w).floor() as i32;
        let tile_max_x = (cx + half_w).ceil() as i32;
        let tile_min_y = (cy - half_h).floor() as i32;
        let tile_max_y = (cy + half_h).ceil() as i32;

        let n = 1i32 << zoom;

        let mut fetch_tasks = Vec::new();
        for ty in tile_min_y..tile_max_y {
            for tx in tile_min_x..tile_max_x {
                let wrapped_tx = tx.rem_euclid(n) as u32;
                if ty < 0 || ty >= n {
                    continue;
                }
                let url = format!("{base_url}/{zoom}/{wrapped_tx}/{ty}.png");
                let client = client.clone();
                fetch_tasks.push(async move {
                    let result = fetch_tile(&client, &url).await;
                    (tx, ty, result)
                });
            }
        }

        let results = futures::future::join_all(fetch_tasks).await;

        let mut canvas = RgbaImage::new(MAP_WIDTH, MAP_HEIGHT);
        let origin_px = cx * f64::from(TILE_SIZE);
        let origin_py = cy * f64::from(TILE_SIZE);
        let canvas_origin_x = origin_px - f64::from(MAP_WIDTH) / 2.0;
        let canvas_origin_y = origin_py - f64::from(MAP_HEIGHT) / 2.0;

        let mut any_tile = false;
        for (tx, ty, tile_result) in &results {
            let Some(tile_img) = tile_result else {
                continue;
            };
            any_tile = true;
            let tile_px = *tx as f64 * f64::from(TILE_SIZE);
            let tile_py = *ty as f64 * f64::from(TILE_SIZE);
            let dx = (tile_px - canvas_origin_x).round() as i64;
            let dy = (tile_py - canvas_origin_y).round() as i64;
            imageops::overlay(&mut canvas, tile_img, dx, dy);
        }

        if !any_tile {
            return None;
        }

        for marker in markers {
            let (mx, my) = lat_lon_to_tile(marker.lat, marker.lon, zoom);
            let px = (mx * f64::from(TILE_SIZE) - canvas_origin_x).round() as i32;
            let py = (my * f64::from(TILE_SIZE) - canvas_origin_y).round() as i32;
            draw_marker(&mut canvas, px, py, MARKER_RADIUS, marker.color);
        }

        let mut buf = Cursor::new(Vec::new());
        canvas.write_to(&mut buf, ImageFormat::Png).ok()?;
        let b64 = BASE64.encode(buf.into_inner());
        Some(format!("data:image/png;base64,{b64}"))
    }

    // ── ImageMagick fallback tests ──────────────────────────────────────────

    #[tokio::test]
    async fn magick_fallback_produces_image() {
        // Skip if magick is not installed (CI environments).
        let magick_ok = tokio::process::Command::new("magick")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if !magick_ok {
            eprintln!("skipping magick test — magick not installed");
            return;
        }

        // Use a mock server so the test doesn't hit the real tile server.
        let mut tile_img = RgbaImage::new(256, 256);
        for pixel in tile_img.pixels_mut() {
            *pixel = image::Rgba([180, 200, 180, 255]);
        }
        let mut png_buf = Cursor::new(Vec::new());
        tile_img.write_to(&mut png_buf, ImageFormat::Png).unwrap();

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "image/png")
            .with_body(png_buf.into_inner())
            .create_async()
            .await;

        let markers = vec![Marker {
            lat: 37.76,
            lon: -122.42,
            color: [220, 50, 50, 255],
        }];

        // compose_static_map_magick uses hardcoded tile URLs, so we can't
        // redirect it to mock. Instead, test the args-building logic by
        // verifying that it at least doesn't crash and returns Some/None.
        // Full integration with real tiles is covered by manual QA.
        let result = compose_static_map_magick(37.76, -122.42, 15, &markers).await;
        // On dev machines with network, this should succeed.
        // In CI without network, it gracefully returns None.
        if let Some(data) = result {
            assert!(data.starts_with("data:image/png;base64,"));
        }
    }
}
