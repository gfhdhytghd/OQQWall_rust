use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use oqqwall_rust_core::Draft;
use oqqwall_rust_drivers::renderer::{
    RenderPreviewHeader, RendererRuntimeConfig, render_preview_png_pages,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    #[serde(default)]
    header: RenderPreviewHeader,
    #[serde(default)]
    config: FixtureConfig,
    draft: Draft,
}

#[derive(Debug, Default, Deserialize)]
struct FixtureConfig {
    canvas_width_px: Option<u32>,
    max_height_px: Option<u32>,
    watermark_text: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let output = args.next().map(PathBuf::from).ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage().into());
    }

    let fixture = serde_json::from_slice::<Fixture>(&fs::read(&input)?)?;
    let mut config = RendererRuntimeConfig::default();
    if let Some(width) = fixture.config.canvas_width_px {
        config.canvas_width_px = width;
    }
    if let Some(height) = fixture.config.max_height_px {
        config.max_height_px = height;
    }
    if let Some(watermark_text) = fixture.config.watermark_text {
        config
            .watermark_text_by_group
            .insert(fixture.header.group_id.clone(), watermark_text);
    }

    let pages = render_preview_png_pages(&fixture.draft, fixture.header, &config)?;
    for (idx, bytes) in pages.into_iter().enumerate() {
        let page_output = if idx == 0 {
            output.clone()
        } else {
            page_output_path(&output, idx + 1)
        };
        if let Some(parent) = page_output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(page_output, bytes)?;
    }
    Ok(())
}

fn usage() -> String {
    "usage: render_fixture <fixture.json> <output.png>".to_string()
}

fn page_output_path(output: &Path, page_number: usize) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let ext = output
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    output.with_file_name(format!("{stem}-page-{page_number:03}.{ext}"))
}
