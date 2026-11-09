use std::hint::black_box;
use std::time::Instant;

use ghostengine::{
    fill_path_even_odd_aa, fill_polygon_aa, stroke_path_aa, Color, Framebuffer, Path, Point,
    StrokeStyle, Transform2D, Vector,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;

fn measure<F>(name: &str, iterations: u32, mut render: F)
where
    F: FnMut(&mut Framebuffer),
{
    let mut target = Framebuffer::new(WIDTH, HEIGHT, Color::BLACK)
        .expect("benchmark framebuffer allocation should succeed");

    for _ in 0..3 {
        render(&mut target);
        black_box(target.pixels());
    }

    let start = Instant::now();
    for _ in 0..iterations {
        target.clear(Color::BLACK);
        render(&mut target);
        black_box(target.pixels());
    }
    let elapsed = start.elapsed();
    let average = elapsed / iterations;

    println!(
        "{name:<28} total={:>9.3} ms  average={:>9.3} ms/frame  throughput={:>8.2} frames/s",
        elapsed.as_secs_f64() * 1_000.0,
        average.as_secs_f64() * 1_000.0,
        1.0 / average.as_secs_f64()
    );
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(20)
        .clamp(1, 10_000);

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

    println!("GhostEngine CPU raster benchmark");
    println!("resolution: {WIDTH}x{HEIGHT} | iterations: {iterations}");
    println!("warm-up: 3 frames per workload");
    println!(
        "build: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!();

    measure("polygon AA (10 vertices)", iterations, |target| {
        fill_polygon_aa(target, &polygon, Color::rgba(40, 170, 245, 210));
    });

    measure("vector path fill AA", iterations, |target| {
        fill_path_even_odd_aa(target, &path, Color::rgba(245, 130, 45, 220), 0.35)
            .expect("benchmark path should be valid");
    });

    measure("vector path stroke AA", iterations, |target| {
        stroke_path_aa(
            target,
            &path,
            Transform2D::IDENTITY,
            StrokeStyle::new(7.0),
            Color::rgba(90, 220, 130, 230),
            0.35,
        )
        .expect("benchmark stroke should be valid");
    });
}
