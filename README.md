# Rust IP Geolocator — Steps API

A minimal backend-only Rust API that calculates the **approximate walking steps** between two places on Earth.

## What It Does

1. You send **two place names** (e.g. `"Paris"` and `"Tokyo"`)
2. The server **geocodes** both places into latitude/longitude coordinates using [Nominatim](https://nominatim.openstreetmap.org/) (OpenStreetMap's free geocoding service)
3. It calculates the **haversine (straight-line) distance** between the two points
4. It converts that distance into an **approximate step count** (assuming ~0.75 meters per step)
5. Returns everything as **JSON**

> **Note:** This uses straight-line (as-the-crow-flies) distance, not actual walking distance. Real walking steps will differ due to roads, terrain, and elevation.

---

## Prerequisites

You need **Rust** installed. If you don't have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then restart your terminal or run:

```bash
source $HOME/.cargo/env
```

Verify it worked:

```bash
rustc --version
cargo --version
```

---

## Project Structure

```
rust-ip-geolocator/
├── Cargo.toml       # Project name, version, dependencies
├── Cargo.lock       # Auto-generated lock file
├── src/
│   └── main.rs      # All application code lives here
└── README.md
```

---

## Step 1: Create the Project

If you're starting from scratch:

```bash
cargo new rust-ip-geolocator
cd rust-ip-geolocator
```

This creates the project folder with a default `Cargo.toml` and `src/main.rs`.

---

## Step 2: Add Dependencies

Open `Cargo.toml` and make the `[dependencies]` section look like this:

```toml
[package]
name = "rust-ip-geolocator"
version = "0.1.0"
edition = "2024"

[dependencies]
actix-web = "4"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### What each dependency does

| Crate | Purpose |
|---|---|
| `actix-web` | Web framework — handles HTTP requests and serves responses |
| `tokio` | Async runtime — actix-web needs an async executor to run on |
| `reqwest` | HTTP client — used to call the Nominatim geocoding API |
| `serde` | Serialization/deserialization — turns Rust structs into JSON and back |
| `serde_json` | JSON support — handles the actual JSON encoding/decoding |

### Install them

```bash
cargo build
```

This downloads and compiles all dependencies. First time takes a few minutes.

---

## Step 3: Write the Code

Replace the contents of `src/main.rs` with the code below. Every section is explained.

```rust
use actix_web::{get, web, App, HttpServer, HttpResponse};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

// ─── Constants ──────────────────────────────────────────────
// Average step length in meters. Change this if you want to
// adjust for taller/shorter people. 0.75m ≈ 2.46 feet.
const AVG_STEP_METERS: f64 = 0.75;

// Earth's mean radius in kilometers.
const EARTH_RADIUS_KM: f64 = 6371.0;

// ─── Request / Response Types ───────────────────────────────

/// The query string the client sends.
/// Example: /steps?from=Paris&to=Tokyo
#[derive(Debug, Deserialize)]
struct Query {
    from: String,
    to: String,
}

/// What Nominatim returns for each search result.
#[derive(Debug, Deserialize)]
struct NominatimPlace {
    lat: String,          // Nominatim returns lat/lon as strings
    lon: String,
    display_name: String, // Human-readable full address
}

/// The JSON response we send back to the client.
#[derive(Debug, Serialize)]
struct Response {
    from_place: String,
    from_coords: Coords,
    to_place: String,
    to_coords: Coords,
    distance_km: f64,
    steps: u64,
}

/// Simple lat/lon pair.
#[derive(Debug, Serialize)]
struct Coords {
    lat: f64,
    lon: f64,
}

// ─── Geocoding ──────────────────────────────────────────────

/// Takes a place name (e.g. "New York") and returns its
/// coordinates and display name using the Nominatim API.
///
/// Nominatim usage policy: https://operations.osmfoundation.org/policies/nominatim/
/// - Must set a valid User-Agent header
/// - Max 1 request per second
async fn geocode(client: &Client, place: &str) -> Result<(Coords, String), String> {
    let url = format!(
        "https://nominatim.openstreetmap.org/search?q={}&format=json&limit=1",
        place
    );

    // Send the request with a descriptive User-Agent (required by Nominatim)
    let resp: Vec<NominatimPlace> = client
        .get(&url)
        .header("User-Agent", "rust-steps-api/0.1.0")
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Parse failed: {e}"))?;

    // Pick the first result (or return an error if nothing found)
    let p = resp
        .first()
        .ok_or_else(|| format!("Place not found: {place}"))?;

    let lat: f64 = p.lat.parse().map_err(|e| format!("Bad lat: {e}"))?;
    let lon: f64 = p.lon.parse().map_err(|e| format!("Bad lon: {e}"))?;

    Ok((Coords { lat, lon }, p.display_name.clone()))
}

// ─── Distance Calculation ───────────────────────────────────

/// Calculates the great-circle distance between two points
/// on Earth using the haversine formula.
///
/// Returns distance in **kilometers**.
///
/// How haversine works:
/// 1. Convert lat/lon from degrees to radians
/// 2. Calculate the difference between the two points
/// 3. Apply the haversine formula: a = sin²(Δlat/2) + cos(lat1) * cos(lat2) * sin²(Δlon/2)
/// 4. c = 2 * atan2(√a, √(1-a))
/// 5. distance = R * c  (where R = Earth's radius)
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

// ─── API Endpoint ───────────────────────────────────────────

/// GET /steps?from=<place>&to=<place>
///
/// Example:
///   curl "http://localhost:8080/steps?from=New+York&to=London"
#[get("/steps")]
async fn steps(query: web::Query<Query>) -> HttpResponse {
    let client = Client::new();

    // Geocode both places
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

    // Calculate distance and steps
    let distance_km = haversine_km(&from_coords, &to_coords);
    let steps_count = (distance_km * 1000.0 / AVG_STEP_METERS).round() as u64;

    // Return JSON
    HttpResponse::Ok().json(Response {
        from_place: from_name,
        from_coords,
        to_place: to_name,
        to_coords,
        distance_km: (distance_km * 100.0).round() / 100.0,
        steps: steps_count,
    })
}

// ─── Server Entry Point ─────────────────────────────────────

/// Starts the actix-web HTTP server on port 8080.
#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Listening on http://127.0.0.1:8080");

    HttpServer::new(|| App::new().service(steps))
        .bind("127.0.0.1:8080")?
        .run()
        .await
}
```

---

## Step 4: Build and Run

```bash
cargo run
```

You should see:

```
Listening on http://127.0.0.1:8080
```

---

## Step 5: Test It

Open a second terminal and try:

```bash
curl "http://localhost:8080/steps?from=Paris&to=Tokyo"
```

You should get back JSON like:

```json
{
  "from_place": "Paris, France",
  "from_coords": {
    "lat": 48.8566,
    "lon": 2.3522
  },
  "to_place": "Tokyo, Japan",
  "to_coords": {
    "lat": 35.6762,
    "lon": 139.6503
  },
  "distance_km": 9713.25,
  "steps": 12951000
}
```

### More examples

```bash
# New York to London
curl "http://localhost:8080/steps?from=New+York&to=London"

# Lagos to Nairobi
curl "http://localhost:8080/steps?from=Lagos&to=Nairobi"

# Sydney to Los Angeles
curl "http://localhost:8080/steps?from=Sydney&to=Los+Angeles"
```

Use `+` or `%20` for spaces in place names.

---

## How It Works (Visual)

```
Client                    Server                     External API
  |                         |                           |
  |  GET /steps?from=X&to=Y |                           |
  | ──────────────────────> |                           |
  |                         |  GET /search?q=X          |
  |                         | ────────────────────────> |
  |                         |  <── coordinates for X    |
  |                         | <──────────────────────── |
  |                         |                           |
  |                         |  GET /search?q=Y          |
  |                         | ────────────────────────> |
  |                         |  <── coordinates for Y    |
  |                         | <──────────────────────── |
  |                         |                           |
  |                         |  haversine(X, Y) → km     |
  |                         |  km → steps               |
  |                         |                           |
  |  { distance, steps }    |                           |
  | <─────────────────────  |                           |
```

---

## Customization

### Change step length

Edit the `AVG_STEP_METERS` constant at the top of `main.rs`:

```rust
// Average adult step ≈ 0.75m
const AVG_STEP_METERS: f64 = 0.75;

// Shorter step ≈ 0.6m
const AVG_STEP_METERS: f64 = 0.6;

// Longer step ≈ 0.9m
const AVG_STEP_METERS: f64 = 0.9;
```

### Change the port

Edit the two `127.0.0.1:8080` references in `main()`:

```rust
// Use port 3000 instead
.bind("127.0.0.1:3000")?
```

### Change the host (listen on all network interfaces)

```rust
.bind("0.0.0.0:8080")?
```

---

## Important Notes

- **Nominatim rate limit:** Max 1 request per second. If you get 429 errors, add a delay between requests.
- **Straight-line distance only:** This is not walking/road distance. For real walking distance you'd need a routing API like OSRM or Google Directions.
- **No rate limiting, no auth, no frontend** — by design. This is a minimal backend service.
- **CORS:** If you want to call this from a browser frontend, you'll need to add CORS headers via the `actix-cors` crate.

---

## Concepts to Learn (Beginner's Guide)

This project touches several concepts. You don't need to master all of them — just understand the **basics** of each. Here's what to learn, in order.

### Web & API Concepts

**1. HTTP & REST**
A client (browser, curl) sends a *request* to a server, which sends back a *response*. Our endpoint is `GET /steps?from=X&to=Y` — that's a REST pattern:
- **GET** = the HTTP method (fetch data, no side effects)
- `/steps` = the route (the resource)
- `?from=X&to=Y` = query parameters (the input)

- Resource: [MDN — HTTP overview](https://developer.mozilla.org/en-US/docs/Web/HTTP/Overview)
- Resource: [REST API tutorial](https://www.restapitutorial.com/)

**2. JSON**
The data format both sides speak. Rules: `{}` objects, `[]` arrays, `"key": value`. Example: `{"lat": 48.85, "lon": 2.35}`. In Rust, `serde` converts between JSON text and Rust structs automatically.
- Resource: [JSON intro — w3schools](https://www.w3schools.com/whatis/whatis_json.asp)
- Resource: [MDN — JSON](https://developer.mozilla.org/en-US/docs/Learn/JavaScript/Objects/JSON)

**3. Geocoding**
Turning a human place name (e.g. `"Paris"`) into coordinates (`lat 48.85, lon 2.35`). The *reverse* (coordinates → name) is called reverse geocoding. We use Nominatim for the forward version.
- Resource: [Nominatim about page](https://nominatim.org/)

**4. Latitude & Longitude**
The global grid for locating any point on Earth:
- **Latitude** (lat): north/south, from −90° (South Pole) to +90° (North Pole)
- **Longitude** (lon): east/west, from −180° to +180°
- One fraction of a degree is a meaningful physical distance (approx 111 km per degree).
- Resource: [Latitude & longitude explained](https://www.geo.fu-berlin.de/en/v/geo-it-examples/latitude-longitude/index.html)

**5. Haversine Formula**
How to get the real distance between two points **on a curved sphere**. You *cannot* just use flat 2D Pythagoras because Earth is round. Haversine accounts for the curvature.
- Resource: [Haversine formula explainer](https://www.movable-type.co.uk/scripts/latlong.html)
- Resource: [Haversine — Wikipedia](https://en.wikipedia.org/wiki/Haversine_formula)

**6. Async / Concurrency**
A server handles many users "at the same time". Instead of waiting idle while a slow API call runs, an **async runtime** (tokio) lets the server do other work. That's why functions are `async` and why we `await` network calls.
- Resource: [What is async programming?](https://developer.mozilla.org/en-US/docs/Learn/JavaScript/Asynchronous/Concepts)
- Rust-specific: [The Rust async book](https://rust-lang.github.io/async-book/)

### Rust-Specific Concepts

**7. Ownership & Borrowing**
Rust's unique memory-safety feature. Every value has one owner; you can *borrow* it with `&` (read-only) or `&mut` (mutable). This is why you see `&client` and `&Coords` everywhere — we're borrowing instead of copying.
- Resource: [The Rust Book — Ownership](https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html)
- Resource: [The Rust Book — References & Borrowing](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html)

**8. Structs & the `derive` Macros**
A **struct** is a group of named fields: `struct Coords { lat: f64, lon: f64 }`. The `#[derive(...)]` attribute auto-generates extra code:
- `Debug` — lets you print it with `{:?}`
- `Serialize` — converts a Rust struct INTO JSON
- `Deserialize` — converts JSON INTO a Rust struct
- Resource: [The Rust Book — Structs](https://doc.rust-lang.org/book/ch05-00-structs.html)
- Resource: [serde derive docs](https://serde.rs/derive.html)

**9. Error Handling — `Result`**
Rust has no exceptions. Functions that can fail return a `Result<T, E>` — either `Ok(value)` or `Err(error)`. We use `.map_err(...)` to convert one error type into another (e.g. a reqwest error into a String message). Every network/parse call "returns a Result".
- Resource: [The Rust Book — Error Handling](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- Resource: [Result type in Rust](https://doc.rust-lang.org/rust-by-example/error/result.html)

**10. Cargo & Crates (`Cargo.toml`)**
`Cargo` is Rust's build tool + package manager. **Crates** are like libraries/packages (npm for JS, pip for Python). `Cargo.toml` lists your dependencies and versions. `cargo build` downloads and compiles them; `cargo run` builds and runs.
- Resource: [The Cargo Book](https://doc.rust-lang.org/cargo/)
- Resource: [crates.io (the package registry)](https://crates.io/)

**11. Attributes/Macros — `#[get("/steps")]`**
The `#[get("/steps")]` above `async fn steps(...)` is an **attribute macro**. It transforms your plain function into an actix-web *handler* — generating all the boilerplate wiring so the framework knows how to route requests to it.
- Resource: [Rust macros overview](https://doc.rust-lang.org/book/ch19-06-macros.html)

### Optional — Go Deeper

- **HTTP headers & User-Agent** — why Nominatim requires a descriptive User-Agent and limits you to 1 req/sec. See [Nominatim usage policy](https://operations.osmfoundation.org/policies/nominatim/).
- **Blocking vs. non-blocking I/O** — why a slow network call in a `sync` function freezes a server. See [Rust async book — async in depth](https://rust-lang.github.io/async-book/02_async_in_depth/01_chapter.html).
- **CORS** — if you later add a browser frontend, you'll need `actix-cors`. See [actix-cors crate](https://docs.rs/actix-cors/).

### Recommended Learning Path

1. Read [The Rust Book chapters 1–9](https://doc.rust-lang.org/book/) (ownership, structs, error handling).
2. Do the [Rustlings exercises](https://github.com/rust-lang/rustlings) to practice.
3. Follow the [actix-web getting started guide](https://actix.rs/) to learn the framework.
4. Read our README's "How It Works" section again — it should all click now.

---

## License

MIT
