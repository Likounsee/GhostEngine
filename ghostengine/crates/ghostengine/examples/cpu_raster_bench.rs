use std::hint::black_box;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ghostengine::{
    blend_rect, draw_text, fill_path_even_odd, fill_path_even_odd_aa, fill_polygon_aa, stroke_path,
    stroke_path_aa, Color, FontFace, Framebuffer, Gradient, GradientStop, Layer, Path, Point, Rect,
    StrokeStyle, Texture, TextureFilter, Transform2D, UiTheme, Vector, WidgetState,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;
const WARMUP_FRAMES: u32 = 5;

#[derive(Clone)]
struct ResultRow {
    name: &'static str,
    iterations: u32,
    elapsed: Duration,
}

impl ResultRow {
    fn average_ms(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1_000.0 / f64::from(self.iterations)
    }

    fn fps(&self) -> f64 {
        1_000.0 / self.average_ms()
    }
}

fn measure<F>(name: &'static str, iterations: u32, mut render: F) -> ResultRow
where
    F: FnMut(&mut Framebuffer),
{
    let mut target = Framebuffer::new(WIDTH, HEIGHT, Color::BLACK)
        .expect("benchmark framebuffer allocation should succeed");

    for _ in 0..WARMUP_FRAMES {
        target.clear(Color::BLACK);
        render(&mut target);
        black_box(target.pixels());
    }

    let start = Instant::now();
    for _ in 0..iterations {
        target.clear(Color::BLACK);
        render(&mut target);
        black_box(target.pixels());
    }

    ResultRow {
        name,
        iterations,
        elapsed: start.elapsed(),
    }
}

fn cpu_name() -> String {
    if let Ok(value) = std::env::var("PROCESSOR_IDENTIFIER") {
        if !value.trim().is_empty() {
            return value;
        }
    }

    #[cfg(target_os = "linux")]
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(model) = cpuinfo.lines().find_map(|line| {
            line.strip_prefix("model name")
                .and_then(|value| value.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        }) {
            return model;
        }
    }

    "Not reported by operating system".to_owned()
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "Unknown".to_owned())
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(500)
        .clamp(1, 100_000);

    let polygon = [
        Point::new(240, 12),
        Point::new(286, 96),
        Point::new(382, 104),
        Point::new(310, 168),
        Point::new(330, 252),
        Point::new(240, 208),
        Point::new(150, 252),
        Point::new(170, 168),
        Point::new(98, 104),
        Point::new(194, 96),
    ];

    let mut path = Path::new();
    path.move_to(Vector::new(80.0, 140.0))
        .cubic_to(
            Vector::new(100.0, 5.0),
            Vector::new(220.0, 5.0),
            Vector::new(240.0, 140.0),
        )
        .cubic_to(
            Vector::new(260.0, 275.0),
            Vector::new(380.0, 275.0),
            Vector::new(400.0, 140.0),
        )
        .line_to(Vector::new(80.0, 140.0))
        .close();

    let gradient = Gradient::new(vec![
        GradientStop {
            position: 0.0,
            color: Color::rgb(35, 110, 230),
        },
        GradientStop {
            position: 0.5,
            color: Color::rgb(50, 210, 160),
        },
        GradientStop {
            position: 1.0,
            color: Color::rgb(245, 130, 45),
        },
    ])
    .expect("benchmark gradient should be valid");

    let texture_pixels = (0..128 * 128)
        .map(|index| {
            let x = index % 128;
            let y = index / 128;
            if (x / 8 + y / 8) % 2 == 0 {
                Color::rgba(50, 150, 240, 220)
            } else {
                Color::rgba(245, 110, 60, 180)
            }
        })
        .collect();
    let texture =
        Texture::new(128, 128, texture_pixels).expect("benchmark texture should be valid");

    let mut background_layer = Layer::new(400, 220, Color::rgba(35, 90, 155, 255))
        .expect("benchmark layer should allocate");
    background_layer.set_position(30, 25);
    let mut foreground_layer = Layer::new(320, 180, Color::rgba(225, 95, 45, 190))
        .expect("benchmark layer should allocate");
    foreground_layer.set_position(90, 55);
    foreground_layer.set_opacity(210);
    let layers = [background_layer, foreground_layer];

    let font = FontFace::from_bytes(include_bytes!(
        "../tests/fixtures/NotoSansArabic[wdth,wght].ttf"
    ))
    .expect("bundled benchmark font should load");
    let theme = UiTheme::default();
    let text = "مرحبا GhostEngine — CPU benchmark";

    println!("GhostEngine — benchmark global CPU");
    println!("Resolution: {WIDTH}x{HEIGHT}");
    println!("Iterations/workload: {iterations} | warm-up: {WARMUP_FRAMES}");
    println!("CPU: {}", cpu_name());
    println!("Rust: {}", rustc_version());
    println!(
        "Build: {}",
        if cfg!(debug_assertions) {
            "debug (non-comparable)"
        } else {
            "release"
        }
    );
    println!();

    let rows = vec![
        measure("Rectangle fill + clipping", iterations, |target| {
            ghostengine::rasterizer::fill_rect(
                target,
                Rect::new(24, 24, 420, 220),
                Color::rgb(45, 130, 220),
            );
        }),
        measure("Rectangle alpha blend", iterations, |target| {
            blend_rect(
                target,
                Rect::new(24, 24, 420, 220),
                Color::rgba(220, 90, 55, 150),
            );
        }),
        measure("Antialiased line", iterations, |target| {
            ghostengine::rasterizer::draw_line_aa(
                target,
                Point::new(12, 12),
                Point::new(468, 258),
                Color::rgba(80, 210, 140, 230),
            );
        }),
        measure("Polygon fill (10 vertices)", iterations, |target| {
            ghostengine::rasterizer::fill_polygon(target, &polygon, Color::rgba(40, 170, 245, 210));
        }),
        measure("Polygon fill AA (10 vertices)", iterations, |target| {
            fill_polygon_aa(target, &polygon, Color::rgba(40, 170, 245, 210));
        }),
        measure("Ellipse fill AA", iterations, |target| {
            ghostengine::rasterizer::fill_ellipse_aa(
                target,
                Point::new(240, 135),
                185,
                105,
                Color::rgba(80, 190, 120, 220),
            );
        }),
        measure("Vector path fill", iterations, |target| {
            fill_path_even_odd(target, &path, Color::rgba(245, 130, 45, 220), 0.35)
                .expect("benchmark path should be valid");
        }),
        measure("Vector path fill AA", iterations, |target| {
            fill_path_even_odd_aa(target, &path, Color::rgba(245, 130, 45, 220), 0.35)
                .expect("benchmark path should be valid");
        }),
        measure("Vector path stroke", iterations, |target| {
            stroke_path(
                target,
                &path,
                Transform2D::IDENTITY,
                StrokeStyle::new(7.0),
                Color::rgba(90, 220, 130, 230),
                0.35,
            )
            .expect("benchmark stroke should be valid");
        }),
        measure("Vector path stroke AA", iterations, |target| {
            stroke_path_aa(
                target,
                &path,
                Transform2D::IDENTITY,
                StrokeStyle::new(7.0),
                Color::rgba(90, 220, 130, 230),
                0.35,
            )
            .expect("benchmark stroke should be valid");
        }),
        measure("Linear gradient", iterations, |target| {
            ghostengine::rasterizer::fill_rect_linear_gradient(
                target,
                Rect::new(12, 12, 456, 246),
                Vector::new(12.0, 12.0),
                Vector::new(468.0, 258.0),
                &gradient,
            );
        }),
        measure("Radial gradient", iterations, |target| {
            ghostengine::rasterizer::fill_rect_radial_gradient(
                target,
                Rect::new(12, 12, 456, 246),
                Vector::new(240.0, 135.0),
                190.0,
                &gradient,
            );
        }),
        measure("Texture nearest", iterations, |target| {
            ghostengine::draw_texture(
                target,
                &texture,
                Rect::new(80, 35, 320, 200),
                TextureFilter::Nearest,
            );
        }),
        measure("Texture bilinear", iterations, |target| {
            ghostengine::draw_texture(
                target,
                &texture,
                Rect::new(80, 35, 320, 200),
                TextureFilter::Bilinear,
            );
        }),
        measure("Layer composition (2 layers)", iterations, |target| {
            ghostengine::compose_layers(target, &layers);
        }),
        measure("Text rasterization (cached)", iterations, |target| {
            draw_text(target, &font, text, Point::new(18, 145), 28.0, Color::WHITE);
        }),
        measure("UI button + text", iterations, |target| {
            ghostengine::draw_button(
                target,
                &font,
                Rect::new(110, 100, 260, 64),
                "مرحبا",
                28.0,
                WidgetState::Hovered,
                &theme,
            );
        }),
    ];

    println!(
        "{:<30} {:>12} {:>14} {:>12}",
        "Workload", "Total (ms)", "ms/frame", "FPS"
    );
    println!("{}", "-".repeat(74));
    for row in &rows {
        println!(
            "{:<30} {:>12.3} {:>14.4} {:>12.2}",
            row.name,
            row.elapsed.as_secs_f64() * 1_000.0,
            row.average_ms(),
            row.fps()
        );
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let report_path = format!("ghostengine-benchmark-{timestamp}.txt");
    let csv_path = format!("ghostengine-benchmark-{timestamp}.csv");

    let mut report = format!(
        "GhostEngine global CPU benchmark\nTimestamp (Unix): {timestamp}\nCPU: {}\nRust: {}\nOS: {} {}\nArchitecture: {}\nResolution: {WIDTH}x{HEIGHT}\nIterations/workload: {iterations}\nWarm-up frames: {WARMUP_FRAMES}\nBuild: {}\n\n",
        cpu_name(),
        rustc_version(),
        std::env::consts::OS,
        std::env::consts::FAMILY,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) { "debug" } else { "release" }
    );
    report.push_str(&format!(
        "{:<30} {:>12} {:>14} {:>12}\n",
        "Workload", "Total (ms)", "ms/frame", "FPS"
    ));
    report.push_str(&format!("{}\n", "-".repeat(74)));

    let mut csv = String::from("workload,iterations,total_ms,average_ms_per_frame,fps\n");
    for row in &rows {
        report.push_str(&format!(
            "{:<30} {:>12.3} {:>14.4} {:>12.2}\n",
            row.name,
            row.elapsed.as_secs_f64() * 1_000.0,
            row.average_ms(),
            row.fps()
        ));
        csv.push_str(&format!(
            "\"{}\",{},{:.3},{:.6},{:.2}\n",
            row.name,
            row.iterations,
            row.elapsed.as_secs_f64() * 1_000.0,
            row.average_ms(),
            row.fps()
        ));
    }

    std::fs::write(&report_path, report).expect("benchmark text report should be writable");
    std::fs::write(&csv_path, csv).expect("benchmark CSV report should be writable");
    println!();
    println!("Reports written:");
    println!("  {report_path}");
    println!("  {csv_path}");
}
