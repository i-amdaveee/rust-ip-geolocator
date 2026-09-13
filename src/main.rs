use actix_web::{get, web, App, HttpServer, HttpResponse};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

const AVG_STEP_METERS: f64 = 0.75;

const EARTH_RADIUS_KM: f64 = 6371.0;

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
struct Query {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
struct NominatimPlace {
    lat: String,
    lon: String,
    display_name: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct Response {
    from_place: String,
    from_coords: Coords,
    to_place: String,
    to_coords: Coords,
    distance_km: f64,
    steps: u64,
}

#[derive(Debug, Serialize, ToSchema)]
struct Coords {
    lat: f64,
    lon: f64,
}

async fn geocode(client: &Client, place: &str) -> Result<(Coords, String), String> {
    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1",
        place
    );

    let resp: Vec<NominatimPlace> = client
        .get(&url)
        .header("User-Agent", "rust-steps-api/0.1.0")
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Parse failed: {e}"))?;

    let p = resp
        .first()
        .ok_or_else(|| format!("Place not found: {place}"))?;

    let lat: f64 = p.lat.parse().map_err(|e| format!("Bad lat: {e}"))?;
    let lon: f64 = p.lon.parse().map_err(|e| format!("Bad lon: {e}"))?;

    Ok((Coords { lat, lon }, p.display_name.clone()))
}

fn haversine_km(a: &Coords, b: &Coords) -> f64 {
    let d_lat = (b.lat - a.lat) * PI / 180.0;
    let d_lon = (b.lon - a.lon) * PI / 180.0;

    let a_val = (d_lat / 2.0).sin().powi(2)
        + a.lat.to_radians().cos()
            * b.lat.to_radians().cos()
            * (d_lon / 2.0).sin().powi(2);

    let c = 2.0 * a_val.sqrt().atan2((1.0 - a_val).sqrt());

    EARTH_RADIUS_KM * c
}

#[derive(OpenApi)]
#[openapi(
    paths(steps),
    components(schemas(Response, Coords, Query)),
    info(
        title = "Rust IP Geolocator — Steps API",
        description = "Calculates the approximate walking steps between two places using Nominatim geocoding and the haversine formula.",
        version = "0.1.0",
        license(name = "MIT")
    )
)]
struct ApiDoc;

#[utoipa::path(
    get,
    path = "/steps",
    params(Query),
    responses(
        (status = 200, description = "Distance and step count between the two places", body = Response),
        (status = 400, description = "Geocoding failed or place not found")
    )
)]
#[get("/steps")]
async fn steps(query: web::Query<Query>) -> HttpResponse {
    let client = Client::new();

    let (from_coords, from_name) = match geocode(&client, &query.from).await {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": e
            }))
        }
    };

    let (to_coords, to_name) = match geocode(&client, &query.to).await {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "error": e
            }))
        }
    };

    let distance_km = haversine_km(&from_coords, &to_coords);
    let steps_count = (distance_km * 1000.0 / AVG_STEP_METERS).round() as u64;

    HttpResponse::Ok().json(Response {
        from_place: from_name,
        from_coords,
        to_place: to_name,
        to_coords,
        distance_km: (distance_km * 100.0).round() / 100.0,
        steps: steps_count,
    })
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let bind_addr = format!("{host}:{port}");

    println!("Listening on http://{bind_addr}");
    println!("Swagger UI at http://{bind_addr}/swagger-ui/");

    HttpServer::new(|| {
        App::new()
            .service(steps)
            .service(
                SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()),
            )
    })
    .bind(&bind_addr)?
    .run()
    .await
}