//! Prints every target a stream would accept, with each camera's formats.

use tinypipewire::{Stream, StreamType};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for (label, kind) in [
        ("audio sources", StreamType::Audio),
        ("cameras", StreamType::Video),
    ] {
        let stream = Stream::new_capture(kind, |_| {})?;
        println!("== {label} ==");

        for target in stream.targets() {
            println!(
                "  {} [{}] {}",
                target.name, target.serial, target.description
            );

            if kind != StreamType::Video {
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
    }
    Ok(())
}
