//! Prints every target a stream would accept, with each camera's formats.

use tinypipewire::Stream;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    print_targets("audio sources", &Stream::audio_capture(|_| {})?, false)?;
    print_targets("cameras", &Stream::video_capture(|_| {})?, true)?;
    Ok(())
}

/// Prints one stream's targets, and for a camera the formats each one offers.
fn print_targets(
    label: &str,
    stream: &Stream,
    with_formats: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("== {label} ==");

    for target in stream.targets() {
        println!(
            "  {} [{}] {}",
            target.name, target.serial, target.description
        );

        if !with_formats {
            continue;
        }
        for format in stream.target_video_formats(Some(&target.name))? {
            let name = format.pixel_format.map_or("?", |f| f.as_str());
            println!(
                "      {name} {}x{}{} @ {:?}",
                format.width,
                format.height,
                if format.is_size_range() { "+" } else { "" },
                format.fps
            );
        }
    }
    Ok(())
}
