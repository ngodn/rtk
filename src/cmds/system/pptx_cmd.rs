//! PowerPoint (.pptx) inspection command.
//!
//! Reads PPTX files (which are ZIP archives of XML) to extract slide content,
//! metadata, and shape information in a token-optimized format.

use crate::core::tracking;
use anyhow::{bail, Context, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read as IoRead;
use std::path::Path;
use zip::ZipArchive;

/// A shape extracted from a slide.
#[derive(Debug)]
struct Shape {
    name: String,
    text: String,
    fill_color: Option<String>,
    _x: Option<i64>,
    _y: Option<i64>,
    font_size: Option<u32>,
}

/// Parse a slide XML and extract shapes with their text content.
fn parse_slide_xml(xml: &str) -> Result<Vec<Shape>> {
    let mut reader = Reader::from_str(xml);
    let mut shapes: Vec<Shape> = Vec::new();

    // State tracking
    let mut in_sp = false;
    let mut in_tx_body = false;
    let mut in_text_run = false;
    let mut current_name = String::new();
    let mut current_text_parts: Vec<String> = Vec::new();
    let mut current_fill: Option<String> = None;
    let mut current_x: Option<i64> = None;
    let mut current_y: Option<i64> = None;
    let mut current_font_size: Option<u32> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "sp" => {
                        in_sp = true;
                        current_name.clear();
                        current_text_parts.clear();
                        current_fill = None;
                        current_x = None;
                        current_y = None;
                        current_font_size = None;
                    }
                    "cNvPr" if in_sp => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"name" {
                                current_name = String::from_utf8_lossy(&attr.value).to_string();
                            }
                        }
                    }
                    "off" if in_sp => {
                        for attr in e.attributes().flatten() {
                            let key = attr.key.as_ref();
                            let val = String::from_utf8_lossy(&attr.value);
                            if key == b"x" {
                                current_x = val.parse().ok();
                            } else if key == b"y" {
                                current_y = val.parse().ok();
                            }
                        }
                    }
                    "srgbClr" if in_sp && !in_tx_body => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"val" {
                                current_fill =
                                    Some(String::from_utf8_lossy(&attr.value).to_string());
                            }
                        }
                    }
                    "txBody" if in_sp => {
                        in_tx_body = true;
                    }
                    "t" if in_tx_body => {
                        in_text_run = true;
                    }
                    "rPr" if in_tx_body => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"sz" {
                                let val = String::from_utf8_lossy(&attr.value);
                                if let Ok(sz) = val.parse::<u32>() {
                                    // OOXML font size is in hundredths of a point
                                    current_font_size = Some(sz / 100);
                                }
                            }
                        }
                    }
                    "defRPr" if in_tx_body && current_font_size.is_none() => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"sz" {
                                let val = String::from_utf8_lossy(&attr.value);
                                if let Ok(sz) = val.parse::<u32>() {
                                    current_font_size = Some(sz / 100);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref t)) if in_text_run => {
                let text = t.unescape().unwrap_or_default().to_string();
                if !text.is_empty() {
                    current_text_parts.push(text);
                }
            }
            Ok(Event::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "sp" => {
                        if in_sp {
                            let text = current_text_parts.join("");
                            shapes.push(Shape {
                                name: current_name.clone(),
                                text,
                                fill_color: current_fill.take(),
                                _x: current_x.take(),
                                _y: current_y.take(),
                                font_size: current_font_size.take(),
                            });
                            in_sp = false;
                            in_tx_body = false;
                            in_text_run = false;
                        }
                    }
                    "txBody" => {
                        in_tx_body = false;
                    }
                    "t" => {
                        in_text_run = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                // Best-effort: stop parsing on error but return what we have
                eprintln!("rtk: pptx xml parse warning: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(shapes)
}

/// Extract the local name from a potentially namespaced XML tag.
/// e.g. b"p:sp" -> "sp", b"a:t" -> "t"
fn local_name(full: &[u8]) -> String {
    let s = std::str::from_utf8(full).unwrap_or("");
    if let Some(pos) = s.rfind(':') {
        s[pos + 1..].to_string()
    } else {
        s.to_string()
    }
}

/// Guess shape type from its name.
fn shape_type(name: &str) -> &str {
    let lower = name.to_lowercase();
    if lower.contains("title")
        || lower.contains("subtitle")
        || lower.contains("text")
        || lower.contains("content")
    {
        "TextBox"
    } else if lower.contains("rect") {
        "Rectangle"
    } else if lower.contains("arrow") || lower.contains("connector") {
        "Arrow"
    } else if lower.contains("picture") || lower.contains("image") {
        "Image"
    } else if lower.contains("table") {
        "Table"
    } else if lower.contains("chart") {
        "Chart"
    } else if lower.contains("group") {
        "Group"
    } else {
        "Shape"
    }
}

/// Infer slide title from shapes (first shape whose name contains "title").
fn slide_title(shapes: &[Shape]) -> String {
    // First try shapes with "title" in name
    for s in shapes {
        if s.name.to_lowercase().contains("title") && !s.text.trim().is_empty() {
            return s.text.trim().to_string();
        }
    }
    // Fall back to first shape with text
    for s in shapes {
        if !s.text.trim().is_empty() {
            return s.text.trim().to_string();
        }
    }
    "(untitled)".to_string()
}

/// Open a PPTX file and return a ZipArchive.
fn open_pptx(path: &Path) -> Result<ZipArchive<File>> {
    let file = File::open(path).with_context(|| format!("Failed to open: {}", path.display()))?;
    ZipArchive::new(file).with_context(|| format!("Not a valid PPTX/ZIP file: {}", path.display()))
}

/// Read a zip entry as a String.
fn read_zip_entry(archive: &mut ZipArchive<File>, name: &str) -> Result<String> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("Missing entry in PPTX: {}", name))?;
    let mut content = String::new();
    entry
        .read_to_string(&mut content)
        .with_context(|| format!("Failed to read: {}", name))?;
    Ok(content)
}

/// List slide entry names in order (ppt/slides/slide1.xml, slide2.xml, ...).
fn slide_entries(archive: &ZipArchive<File>) -> Vec<String> {
    let mut slides: Vec<String> = archive
        .file_names()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .map(|s| s.to_string())
        .collect();

    // Sort by slide number
    slides.sort_by_key(|name| {
        name.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u32>()
            .unwrap_or(0)
    });
    slides
}

/// Extract slide number from entry name.
fn slide_number(entry: &str) -> u32 {
    entry
        .trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(0)
}

/// Parse metadata from docProps/core.xml.
fn parse_core_metadata(xml: &str) -> BTreeMap<String, String> {
    let mut meta = BTreeMap::new();
    let mut reader = Reader::from_str(xml);
    let mut current_tag = String::new();
    let mut in_element = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "title" | "creator" | "lastModifiedBy" | "created" | "modified" => {
                        current_tag = local.to_string();
                        in_element = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref t)) if in_element => {
                if let Ok(text) = t.unescape() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        meta.insert(current_tag.clone(), text);
                    }
                }
            }
            Ok(Event::End(_)) => {
                in_element = false;
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    meta
}

// ── Subcommand implementations ──────────────────────────────────────────

/// `rtk pptx info <file>` - Show slide count, dimensions, metadata.
pub fn run_info(file: &Path, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut archive = open_pptx(file)?;
    let slides = slide_entries(&archive);
    let slide_count = slides.len();

    let mut output_lines: Vec<String> = Vec::new();
    output_lines.push(format!("Slides: {}", slide_count));

    // Try to get metadata from docProps/core.xml
    if let Ok(core_xml) = read_zip_entry(&mut archive, "docProps/core.xml") {
        let meta = parse_core_metadata(&core_xml);
        if let Some(title) = meta.get("title") {
            output_lines.push(format!("Title: {}", title));
        }
        if let Some(author) = meta.get("creator") {
            output_lines.push(format!("Author: {}", author));
        }
        if let Some(modified_by) = meta.get("lastModifiedBy") {
            output_lines.push(format!("Last modified by: {}", modified_by));
        }
    }

    // Try to get dimensions from ppt/presentation.xml
    if let Ok(pres_xml) = read_zip_entry(&mut archive, "ppt/presentation.xml") {
        if let Some(dims) = parse_slide_dimensions(&pres_xml) {
            output_lines.push(format!("Dimensions: {}x{}", dims.0, dims.1));
        }
    }

    let output = output_lines.join("\n");
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Entries in PPTX: {}", archive.len());
    }

    let raw_estimate = format!("PPTX file with {} slides, full XML content", slide_count);
    timer.track(
        &format!("cat {}", file.display()),
        "rtk pptx info",
        &raw_estimate,
        &output,
    );
    Ok(())
}

/// Parse slide dimensions from presentation.xml.
/// Returns (width_pt, height_pt) in approximate points.
fn parse_slide_dimensions(xml: &str) -> Option<(u32, u32)> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                if local.as_str() == "sldSz" {
                    let mut cx = None;
                    let mut cy = None;
                    for attr in e.attributes().flatten() {
                        let val = String::from_utf8_lossy(&attr.value);
                        if attr.key.as_ref() == b"cx" {
                            cx = val.parse::<u64>().ok();
                        } else if attr.key.as_ref() == b"cy" {
                            cy = val.parse::<u64>().ok();
                        }
                    }
                    if let (Some(w), Some(h)) = (cx, cy) {
                        // EMU to points: 1 point = 12700 EMU
                        return Some(((w / 12700) as u32, (h / 12700) as u32));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    None
}

/// `rtk pptx slides <file>` - List all slides with titles.
pub fn run_slides(file: &Path, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut archive = open_pptx(file)?;
    let slides = slide_entries(&archive);

    if slides.is_empty() {
        bail!("No slides found in {}", file.display());
    }

    let mut output_lines: Vec<String> = Vec::new();

    for entry in &slides {
        let num = slide_number(entry);
        let xml = read_zip_entry(&mut archive, entry)
            .with_context(|| format!("Failed to read {}", entry))?;
        let shapes = parse_slide_xml(&xml).unwrap_or_default();
        let title = slide_title(&shapes);
        output_lines.push(format!("{:>2}: {}", num, title));
    }

    let output = output_lines.join("\n");
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Parsed {} slides", slides.len());
    }

    let raw_estimate = format!(
        "PPTX with {} slides, full XML (~{}KB per slide)",
        slides.len(),
        slides.len() * 5
    );
    timer.track(
        &format!("cat {}", file.display()),
        "rtk pptx slides",
        &raw_estimate,
        &output,
    );
    Ok(())
}

/// `rtk pptx read <file> <slide>` - Read slide details.
/// `slide` can be "3" or "3-5".
pub fn run_read(file: &Path, slide_spec: &str, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut archive = open_pptx(file)?;
    let all_slides = slide_entries(&archive);

    let (start, end) = parse_slide_range(slide_spec)?;

    let mut output_lines: Vec<String> = Vec::new();
    let mut raw_size_estimate = 0usize;

    for slide_num in start..=end {
        let entry = format!("ppt/slides/slide{}.xml", slide_num);
        if !all_slides.contains(&entry) {
            output_lines.push(format!("Slide {}: (not found)", slide_num));
            continue;
        }

        let xml = read_zip_entry(&mut archive, &entry)
            .with_context(|| format!("Failed to read slide {}", slide_num))?;
        raw_size_estimate += xml.len();
        let shapes = parse_slide_xml(&xml).unwrap_or_default();
        let title = slide_title(&shapes);

        if !output_lines.is_empty() {
            output_lines.push(String::new());
        }
        output_lines.push(format!("Slide {}: {}", slide_num, title));
        output_lines.push(String::new());

        for shape in &shapes {
            let stype = shape_type(&shape.name);
            let mut desc = format!("[{}]", stype);

            if !shape.text.trim().is_empty() {
                desc.push_str(&format!(" \"{}\"", shape.text.trim()));
            }

            if let Some(sz) = shape.font_size {
                if sz > 0 {
                    desc.push_str(&format!(" ({}pt)", sz));
                }
            }

            if let Some(ref fill) = shape.fill_color {
                desc.push_str(&format!(" fill:#{}", fill));
            }

            // Skip shapes with no useful content
            if shape.text.trim().is_empty() && shape.fill_color.is_none() {
                continue;
            }

            output_lines.push(desc);
        }
    }

    let output = output_lines.join("\n");
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Raw XML size: ~{}KB", raw_size_estimate / 1024);
    }

    let raw_estimate = "x".repeat(raw_size_estimate.max(1));
    timer.track(
        &format!("cat {}", file.display()),
        "rtk pptx read",
        &raw_estimate,
        &output,
    );
    Ok(())
}

/// `rtk pptx find <file> <query>` - Find shapes containing text.
pub fn run_find(file: &Path, query: &str, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut archive = open_pptx(file)?;
    let slides = slide_entries(&archive);

    let query_lower = query.to_lowercase();
    let mut output_lines: Vec<String> = Vec::new();
    let mut raw_size_estimate = 0usize;

    for entry in &slides {
        let num = slide_number(entry);
        let xml = read_zip_entry(&mut archive, entry)
            .with_context(|| format!("Failed to read {}", entry))?;
        raw_size_estimate += xml.len();
        let shapes = parse_slide_xml(&xml).unwrap_or_default();

        for (i, shape) in shapes.iter().enumerate() {
            if shape.text.to_lowercase().contains(&query_lower) {
                let text_preview = if shape.text.len() > 80 {
                    format!("{}...", &shape.text[..77])
                } else {
                    shape.text.clone()
                };
                output_lines.push(format!(
                    "Slide {}, Shape {}: \"{}\"",
                    num,
                    i + 1,
                    text_preview.trim()
                ));
            }
        }
    }

    if output_lines.is_empty() {
        output_lines.push(format!("No matches for \"{}\"", query));
    }

    let output = output_lines.join("\n");
    println!("{}", output);

    if verbose > 0 {
        eprintln!(
            "Searched {} slides, {} matches",
            slides.len(),
            output_lines.len()
        );
    }

    let raw_estimate = "x".repeat(raw_size_estimate.max(1));
    timer.track(
        &format!("cat {}", file.display()),
        "rtk pptx find",
        &raw_estimate,
        &output,
    );
    Ok(())
}

/// Parse slide range: "3" -> (3, 3), "3-5" -> (3, 5).
fn parse_slide_range(spec: &str) -> Result<(u32, u32)> {
    if let Some((start_str, end_str)) = spec.split_once('-') {
        let start: u32 = start_str
            .parse()
            .with_context(|| format!("Invalid slide number: {}", start_str))?;
        let end: u32 = end_str
            .parse()
            .with_context(|| format!("Invalid slide number: {}", end_str))?;
        if start > end {
            bail!("Invalid range: {} > {}", start, end);
        }
        Ok((start, end))
    } else {
        let num: u32 = spec
            .parse()
            .with_context(|| format!("Invalid slide number: {}", spec))?;
        Ok((num, num))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_slide_range_single() {
        let (start, end) = parse_slide_range("3").expect("should parse");
        assert_eq!(start, 3);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_parse_slide_range_range() {
        let (start, end) = parse_slide_range("3-5").expect("should parse");
        assert_eq!(start, 3);
        assert_eq!(end, 5);
    }

    #[test]
    fn test_parse_slide_range_invalid() {
        assert!(parse_slide_range("5-3").is_err());
        assert!(parse_slide_range("abc").is_err());
    }

    #[test]
    fn test_local_name() {
        assert_eq!(local_name(b"p:sp").as_str(), "sp");
        assert_eq!(local_name(b"a:t").as_str(), "t");
        assert_eq!(local_name(b"sldSz").as_str(), "sldSz");
    }

    #[test]
    fn test_shape_type_detection() {
        assert_eq!(shape_type("Title 1"), "TextBox");
        assert_eq!(shape_type("Rectangle 5"), "Rectangle");
        assert_eq!(shape_type("Content Placeholder 1"), "TextBox");
        assert_eq!(shape_type("Arrow Connector 3"), "Arrow");
        assert_eq!(shape_type("Freeform 7"), "Shape");
    }

    #[test]
    fn test_parse_slide_xml_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
       xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:xfrm><a:off x="500" y="200"/><a:ext cx="8000" cy="600"/></a:xfrm>
        </p:spPr>
        <p:txBody>
          <a:p><a:r><a:rPr sz="2400"/><a:t>Hello World</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr><p:cNvPr id="3" name="Rectangle 2"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>
        <p:spPr>
          <a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>
        </p:spPr>
        <p:txBody>
          <a:p><a:r><a:t>Ordered</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

        let shapes = parse_slide_xml(xml).expect("should parse");
        assert_eq!(shapes.len(), 2);

        assert_eq!(shapes[0].name, "Title 1");
        assert_eq!(shapes[0].text, "Hello World");
        assert_eq!(shapes[0].font_size, Some(24));

        assert_eq!(shapes[1].name, "Rectangle 2");
        assert_eq!(shapes[1].text, "Ordered");
        assert_eq!(shapes[1].fill_color.as_deref(), Some("4472C4"));
    }

    #[test]
    fn test_parse_slide_xml_empty() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree></p:spTree></p:cSld>
</p:sld>"#;

        let shapes = parse_slide_xml(xml).expect("should parse");
        assert!(shapes.is_empty());
    }

    #[test]
    fn test_slide_title_extraction() {
        let shapes = vec![
            Shape {
                name: "Rectangle 1".to_string(),
                text: "Some box".to_string(),
                fill_color: None,
                _x: None,
                _y: None,
                font_size: None,
            },
            Shape {
                name: "Title 1".to_string(),
                text: "My Slide Title".to_string(),
                fill_color: None,
                _x: None,
                _y: None,
                font_size: None,
            },
        ];
        assert_eq!(slide_title(&shapes), "My Slide Title");
    }

    #[test]
    fn test_slide_title_fallback() {
        let shapes = vec![Shape {
            name: "Rectangle 1".to_string(),
            text: "Fallback text".to_string(),
            fill_color: None,
            _x: None,
            _y: None,
            font_size: None,
        }];
        assert_eq!(slide_title(&shapes), "Fallback text");
    }

    #[test]
    fn test_parse_core_metadata() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
                   xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title>Test Presentation</dc:title>
  <dc:creator>John Doe</dc:creator>
  <cp:lastModifiedBy>Jane Smith</cp:lastModifiedBy>
</cp:coreProperties>"#;

        let meta = parse_core_metadata(xml);
        assert_eq!(
            meta.get("title").map(|s| s.as_str()),
            Some("Test Presentation")
        );
        assert_eq!(meta.get("creator").map(|s| s.as_str()), Some("John Doe"));
        assert_eq!(
            meta.get("lastModifiedBy").map(|s| s.as_str()),
            Some("Jane Smith")
        );
    }

    #[test]
    fn test_parse_slide_dimensions() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldSz cx="12192000" cy="6858000"/>
</p:presentation>"#;

        let dims = parse_slide_dimensions(xml);
        assert!(dims.is_some());
        let (w, h) = dims.expect("should have dims");
        // 12192000 / 12700 = 960, 6858000 / 12700 = 540
        assert_eq!(w, 960);
        assert_eq!(h, 540);
    }
}
