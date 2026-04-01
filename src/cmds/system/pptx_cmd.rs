//! PowerPoint (.pptx) inspection and modification command.
//!
//! Reads and writes PPTX files (which are ZIP archives of XML) to extract and
//! modify slide content, metadata, and shape information in a token-optimized format.

use crate::core::tracking;
use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as IoRead, Write as IoWrite};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

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
                    let truncated: String = shape.text.chars().take(77).collect();
                    format!("{}...", truncated)
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

// ── Write helpers ──────────────────────────────────────────────────────

lazy_static! {
    /// Match a cNvPr element with a specific name attribute.
    /// Captures: (1) everything before the name value, (2) the name value.
    static ref CNVPR_NAME_RE: Regex =
        Regex::new(r#"<[^>]*cNvPr[^>]*\bname="([^"]*)"[^>]*/?\s*>"#).expect("valid regex");
}

/// Copy a PPTX zip, applying a transformation function to specific entries.
/// `transform` is called for each entry name; if it returns Some(new_content),
/// that content replaces the original. If it returns None, the entry is copied as-is.
/// If `skip_entries` contains the entry name, the entry is omitted entirely.
fn rewrite_pptx<F>(path: &Path, skip_entries: &[&str], transform: F) -> Result<()>
where
    F: Fn(&str, &[u8]) -> Result<Option<Vec<u8>>>,
{
    let file = File::open(path).with_context(|| format!("Failed to open: {}", path.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("Not a valid PPTX: {}", path.display()))?;

    let tmp_path = path.with_extension("pptx.tmp");
    let tmp_file = File::create(&tmp_path)
        .with_context(|| format!("Failed to create temp file: {}", tmp_path.display()))?;
    let mut writer = ZipWriter::new(tmp_file);

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .with_context(|| format!("Failed to read zip entry {}", i))?;
        let name = entry.name().to_string();

        if skip_entries.contains(&name.as_str()) {
            continue;
        }

        let mut raw = Vec::new();
        entry
            .read_to_end(&mut raw)
            .with_context(|| format!("Failed to read entry: {}", name))?;

        let options = SimpleFileOptions::default().compression_method(entry.compression());

        let final_bytes = transform(&name, &raw)?;

        writer
            .start_file(&name, options)
            .with_context(|| format!("Failed to start writing entry: {}", name))?;
        writer
            .write_all(final_bytes.as_deref().unwrap_or(&raw))
            .with_context(|| format!("Failed to write entry: {}", name))?;
    }

    writer.finish().context("Failed to finalize PPTX zip")?;

    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("Failed to replace original file: {}", path.display()))?;

    Ok(())
}

/// Find the XML range of a shape with the given name in slide XML.
/// Returns the byte range of the `<p:sp>...</p:sp>` block and the shape's XML.
fn find_shape_xml_by_name<'a>(xml: &'a str, shape_name: &str) -> Option<(usize, usize, &'a str)> {
    // We need to find the <p:sp> block containing a cNvPr with the matching name.
    // Strategy: find all <p:sp> blocks, check each for the name.
    let sp_open_tag = "<p:sp>";
    let sp_open_tag_with_attrs = "<p:sp ";
    let sp_close_tag = "</p:sp>";

    let mut search_from = 0;
    loop {
        // Find next <p:sp> or <p:sp ...>
        let sp_start = {
            let a = xml[search_from..]
                .find(sp_open_tag)
                .map(|p| search_from + p);
            let b = xml[search_from..]
                .find(sp_open_tag_with_attrs)
                .map(|p| search_from + p);
            match (a, b) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (Some(x), None) => Some(x),
                (None, Some(y)) => Some(y),
                (None, None) => None,
            }
        };

        let sp_start = sp_start?;

        let sp_end = match xml[sp_start..].find(sp_close_tag) {
            Some(p) => sp_start + p + sp_close_tag.len(),
            None => return None,
        };

        let block = &xml[sp_start..sp_end];

        // Check if this block contains a cNvPr with the target name
        for caps in CNVPR_NAME_RE.captures_iter(block) {
            if let Some(m) = caps.get(1) {
                if m.as_str() == shape_name {
                    return Some((sp_start, sp_end, block));
                }
            }
        }

        search_from = sp_end;
    }
}

/// Replace all `<a:t>` content within a shape XML block with new text.
/// The first `<a:t>` gets the full text; subsequent ones are emptied.
/// This preserves all formatting (font, size, color, etc.).
fn replace_text_in_shape_xml(shape_xml: &str, new_text: &str) -> String {
    lazy_static! {
        static ref AT_RE: Regex = Regex::new(r"(<a:t>)(.*?)(</a:t>)").expect("valid regex");
    }

    let mut first = true;
    let result = AT_RE.replace_all(shape_xml, |caps: &regex::Captures| {
        if first {
            first = false;
            format!(
                "{}{}{}",
                &caps[1],
                quick_xml::escape::escape(new_text),
                &caps[3]
            )
        } else {
            format!("{}{}", &caps[1], &caps[3])
        }
    });
    result.to_string()
}

/// Set or replace the solid fill color in a shape XML block.
fn set_fill_in_shape_xml(shape_xml: &str, hex_color: &str) -> String {
    let color = hex_color.trim_start_matches('#');
    let fill_xml = format!("<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>", color);

    // Check if there's already a <a:solidFill> inside <p:spPr>
    lazy_static! {
        static ref SOLID_FILL_RE: Regex =
            Regex::new(r"<a:solidFill>.*?</a:solidFill>").expect("valid regex");
        static ref SPPR_OPEN_RE: Regex = Regex::new(r"<p:spPr[^>]*>").expect("valid regex");
    }

    // If there's already a solidFill, replace it
    if SOLID_FILL_RE.is_match(shape_xml) {
        return SOLID_FILL_RE
            .replace(shape_xml, fill_xml.as_str())
            .to_string();
    }

    // Otherwise, insert after <p:spPr...>
    if let Some(m) = SPPR_OPEN_RE.find(shape_xml) {
        let insert_pos = m.end();
        let mut result = String::with_capacity(shape_xml.len() + fill_xml.len());
        result.push_str(&shape_xml[..insert_pos]);
        result.push_str(&fill_xml);
        result.push_str(&shape_xml[insert_pos..]);
        return result;
    }

    // Fallback: return unchanged
    shape_xml.to_string()
}

// ── Write subcommand implementations ───────────────────────────────────

/// `rtk pptx set-text <file> <slide> <shape-name> <new-text>`
pub fn run_set_text(
    file: &Path,
    slide: u32,
    shape_name: &str,
    new_text: &str,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let slide_entry = format!("ppt/slides/slide{}.xml", slide);
    let shape_name_owned = shape_name.to_string();
    let new_text_owned = new_text.to_string();

    rewrite_pptx(file, &[], |name, raw| {
        if name == slide_entry {
            let xml =
                std::str::from_utf8(raw).with_context(|| format!("Invalid UTF-8 in {}", name))?;

            let (start, end, shape_block) = find_shape_xml_by_name(xml, &shape_name_owned)
                .with_context(|| {
                    format!("Shape '{}' not found on slide {}", shape_name_owned, slide)
                })?;

            let new_block = replace_text_in_shape_xml(shape_block, &new_text_owned);
            let mut result = String::with_capacity(xml.len());
            result.push_str(&xml[..start]);
            result.push_str(&new_block);
            result.push_str(&xml[end..]);
            Ok(Some(result.into_bytes()))
        } else {
            Ok(None)
        }
    })?;

    let output = format!(
        "Set text on slide {} shape '{}' to '{}'",
        slide, shape_name, new_text
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx set-text {}", file.display()),
        "rtk pptx set-text",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// `rtk pptx set-fill <file> <slide> <shape-name> <hex-color>`
pub fn run_set_fill(
    file: &Path,
    slide: u32,
    shape_name: &str,
    hex_color: &str,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let slide_entry = format!("ppt/slides/slide{}.xml", slide);
    let shape_name_owned = shape_name.to_string();
    let hex_color_owned = hex_color.to_string();

    rewrite_pptx(file, &[], |name, raw| {
        if name == slide_entry {
            let xml =
                std::str::from_utf8(raw).with_context(|| format!("Invalid UTF-8 in {}", name))?;

            let (start, end, shape_block) = find_shape_xml_by_name(xml, &shape_name_owned)
                .with_context(|| {
                    format!("Shape '{}' not found on slide {}", shape_name_owned, slide)
                })?;

            let new_block = set_fill_in_shape_xml(shape_block, &hex_color_owned);
            let mut result = String::with_capacity(xml.len());
            result.push_str(&xml[..start]);
            result.push_str(&new_block);
            result.push_str(&xml[end..]);
            Ok(Some(result.into_bytes()))
        } else {
            Ok(None)
        }
    })?;

    let color = hex_color.trim_start_matches('#');
    let output = format!(
        "Set fill on slide {} shape '{}' to #{}",
        slide, shape_name, color
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx set-fill {}", file.display()),
        "rtk pptx set-fill",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// `rtk pptx delete-slide <file> <slide-number>`
pub fn run_delete_slide(file: &Path, slide: u32, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    // Verify the slide exists first
    {
        let mut archive = open_pptx(file)?;
        let slides = slide_entries(&archive);
        let entry_name = format!("ppt/slides/slide{}.xml", slide);
        if !slides.contains(&entry_name) {
            bail!(
                "Slide {} not found in {} (has {} slides)",
                slide,
                file.display(),
                slides.len()
            );
        }
        // Don't allow deleting the last slide
        if slides.len() <= 1 {
            bail!("Cannot delete the only slide in the presentation");
        }
        // Read presentation.xml to find the rId for this slide
        let _pres_xml = read_zip_entry(&mut archive, "ppt/presentation.xml")?;
    }

    let slide_entry = format!("ppt/slides/slide{}.xml", slide);
    let slide_rels_entry = format!("ppt/slides/_rels/slide{}.xml.rels", slide);

    // We need to:
    // 1. Remove the slide XML and its rels
    // 2. Update ppt/presentation.xml to remove the <p:sldIdLst> entry
    // 3. Update [Content_Types].xml to remove the slide's override
    // 4. Update ppt/_rels/presentation.xml.rels to remove the relationship

    // First, find the rId for this slide in presentation.xml.rels
    let rid_to_remove = {
        let mut archive = open_pptx(file)?;
        let rels_xml = read_zip_entry(&mut archive, "ppt/_rels/presentation.xml.rels")?;
        find_rid_for_slide(&rels_xml, slide)?
    };

    let skip = vec![slide_entry.as_str(), slide_rels_entry.as_str()];

    rewrite_pptx(file, &skip, |name, raw| match name {
        "ppt/presentation.xml" => {
            let xml = std::str::from_utf8(raw).context("Invalid UTF-8 in presentation.xml")?;
            let updated = remove_slide_from_presentation_xml(xml, &rid_to_remove)?;
            Ok(Some(updated.into_bytes()))
        }
        "[Content_Types].xml" => {
            let xml = std::str::from_utf8(raw).context("Invalid UTF-8 in [Content_Types].xml")?;
            let updated = remove_slide_from_content_types(xml, slide);
            Ok(Some(updated.into_bytes()))
        }
        "ppt/_rels/presentation.xml.rels" => {
            let xml = std::str::from_utf8(raw).context("Invalid UTF-8 in presentation.xml.rels")?;
            let updated = remove_relationship(xml, &rid_to_remove);
            Ok(Some(updated.into_bytes()))
        }
        _ => Ok(None),
    })?;

    let output = format!("Deleted slide {} from {}", slide, file.display());
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx delete-slide {}", file.display()),
        "rtk pptx delete-slide",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// Find the relationship ID (rId) for a given slide number in presentation.xml.rels.
fn find_rid_for_slide(rels_xml: &str, slide: u32) -> Result<String> {
    lazy_static! {
        static ref REL_RE: Regex = Regex::new(
            r#"<Relationship[^>]*\bId="([^"]*)"[^>]*Target="slides/slide(\d+)\.xml"[^>]*/?\s*>"#
        )
        .expect("valid regex");
        // Also match when Target comes before Id
        static ref REL_RE2: Regex = Regex::new(
            r#"<Relationship[^>]*Target="slides/slide(\d+)\.xml"[^>]*\bId="([^"]*)"[^>]*/?\s*>"#
        )
        .expect("valid regex");
    }

    for caps in REL_RE.captures_iter(rels_xml) {
        if let (Some(rid), Some(num)) = (caps.get(1), caps.get(2)) {
            if num.as_str().parse::<u32>().ok() == Some(slide) {
                return Ok(rid.as_str().to_string());
            }
        }
    }
    for caps in REL_RE2.captures_iter(rels_xml) {
        if let (Some(num), Some(rid)) = (caps.get(1), caps.get(2)) {
            if num.as_str().parse::<u32>().ok() == Some(slide) {
                return Ok(rid.as_str().to_string());
            }
        }
    }

    bail!("Could not find relationship ID for slide {}", slide)
}

/// Remove the `<p:sldId>` entry for the given rId from presentation.xml.
fn remove_slide_from_presentation_xml(xml: &str, rid: &str) -> Result<String> {
    lazy_static! {
        static ref SLDID_RE: Regex = Regex::new(r#"<p:sldId[^>]*/?\s*>"#).expect("valid regex");
    }

    // Remove any <p:sldId ... r:id="rIdXX" .../> that references our rid
    let pattern = format!(r#"<p:sldId[^>]*r:id="{}"\s*/?\s*>"#, regex::escape(rid));
    let specific_re = Regex::new(&pattern).context("Failed to build slide removal regex")?;
    let result = specific_re.replace(xml, "").to_string();
    Ok(result)
}

/// Remove the slide override from [Content_Types].xml.
fn remove_slide_from_content_types(xml: &str, slide: u32) -> String {
    let pattern = format!(
        r#"<Override[^>]*PartName="/ppt/slides/slide{}\.xml"[^>]*/?\s*>"#,
        slide
    );
    if let Ok(re) = Regex::new(&pattern) {
        re.replace(xml, "").to_string()
    } else {
        xml.to_string()
    }
}

/// Remove a <Relationship> entry by its Id from a .rels file.
fn remove_relationship(xml: &str, rid: &str) -> String {
    let pattern = format!(
        r#"<Relationship[^>]*\bId="{}"\s*[^>]*/?\s*>"#,
        regex::escape(rid)
    );
    if let Ok(re) = Regex::new(&pattern) {
        re.replace(xml, "").to_string()
    } else {
        xml.to_string()
    }
}

/// `rtk pptx move-slide <file> <from> <to>`
pub fn run_move_slide(file: &Path, from: u32, to: u32, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if from == to {
        println!("Slide {} is already at position {}", from, to);
        return Ok(());
    }

    // Verify slide count
    let slide_count = {
        let archive = open_pptx(file)?;
        let slides = slide_entries(&archive);
        let count = slides.len() as u32;
        if from < 1 || from > count {
            bail!("Slide {} out of range (1-{})", from, count);
        }
        if to < 1 || to > count {
            bail!("Destination {} out of range (1-{})", to, count);
        }
        count
    };

    // The slide order is determined by <p:sldIdLst> in presentation.xml.
    // We need to reorder the <p:sldId> elements.
    rewrite_pptx(file, &[], |name, raw| {
        if name == "ppt/presentation.xml" {
            let xml = std::str::from_utf8(raw).context("Invalid UTF-8 in presentation.xml")?;
            let updated = reorder_slides_in_presentation_xml(xml, from, to, slide_count)?;
            Ok(Some(updated.into_bytes()))
        } else {
            Ok(None)
        }
    })?;

    let output = format!(
        "Moved slide {} to position {} in {}",
        from,
        to,
        file.display()
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx move-slide {}", file.display()),
        "rtk pptx move-slide",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// Reorder `<p:sldId>` entries within `<p:sldIdLst>` in presentation.xml.
fn reorder_slides_in_presentation_xml(
    xml: &str,
    from: u32,
    to: u32,
    _slide_count: u32,
) -> Result<String> {
    lazy_static! {
        static ref SLDIDLST_RE: Regex =
            Regex::new(r"(?s)<p:sldIdLst>(.*?)</p:sldIdLst>").expect("valid regex");
        static ref SLDID_ENTRY_RE: Regex =
            Regex::new(r#"<p:sldId[^>]*/?\s*>"#).expect("valid regex");
    }

    let list_match = SLDIDLST_RE
        .captures(xml)
        .context("Could not find <p:sldIdLst> in presentation.xml")?;
    let list_content = list_match.get(1).context("Empty sldIdLst")?.as_str();

    // Collect all <p:sldId .../> entries in order
    let entries: Vec<&str> = SLDID_ENTRY_RE
        .find_iter(list_content)
        .map(|m| m.as_str())
        .collect();

    if entries.is_empty() {
        bail!("No slide entries found in <p:sldIdLst>");
    }

    let from_idx = (from as usize)
        .checked_sub(1)
        .context("Invalid from index")?;
    let to_idx = (to as usize).checked_sub(1).context("Invalid to index")?;

    if from_idx >= entries.len() {
        bail!(
            "Slide {} out of range (presentation has {} slides in sldIdLst)",
            from,
            entries.len()
        );
    }

    let mut reordered = entries.clone();
    let item = reordered.remove(from_idx);
    let insert_at = to_idx.min(reordered.len());
    reordered.insert(insert_at, item);

    // Rebuild the sldIdLst content
    let new_list_content = reordered.join("\n    ");
    let new_list = format!("<p:sldIdLst>\n    {}\n  </p:sldIdLst>", new_list_content);

    let full_match = list_match.get(0).context("regex match")?;
    let mut result = String::with_capacity(xml.len());
    result.push_str(&xml[..full_match.start()]);
    result.push_str(&new_list);
    result.push_str(&xml[full_match.end()..]);

    Ok(result)
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

    // ── Write function tests ───────────────────────────────────────────

    #[test]
    fn test_find_shape_xml_by_name() {
        let xml = r#"<p:sld><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/></p:nvSpPr><p:txBody><a:t>Hello</a:t></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:cNvPr id="3" name="Rectangle 2"/></p:nvSpPr><p:txBody><a:t>World</a:t></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;

        let result = find_shape_xml_by_name(xml, "Title 1");
        assert!(result.is_some());
        let (_, _, block) = result.expect("should find shape");
        assert!(block.contains("Title 1"));
        assert!(block.contains("Hello"));

        let result = find_shape_xml_by_name(xml, "Rectangle 2");
        assert!(result.is_some());
        let (_, _, block) = result.expect("should find shape");
        assert!(block.contains("Rectangle 2"));
        assert!(block.contains("World"));

        let result = find_shape_xml_by_name(xml, "Nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_replace_text_in_shape_xml() {
        let shape = r#"<p:sp><p:txBody><a:p><a:r><a:rPr sz="2400"/><a:t>Old Text</a:t></a:r></a:p></p:txBody></p:sp>"#;
        let result = replace_text_in_shape_xml(shape, "New Text");
        assert!(result.contains("<a:t>New Text</a:t>"));
        assert!(!result.contains("Old Text"));
        // Formatting preserved
        assert!(result.contains(r#"sz="2400""#));
    }

    #[test]
    fn test_replace_text_multiple_runs() {
        let shape = r#"<p:sp><p:txBody><a:p><a:r><a:t>Part 1</a:t></a:r><a:r><a:t>Part 2</a:t></a:r></a:p></p:txBody></p:sp>"#;
        let result = replace_text_in_shape_xml(shape, "Full Replacement");
        assert!(result.contains("<a:t>Full Replacement</a:t>"));
        // Second <a:t> should be emptied
        assert!(result.contains("<a:t></a:t>"));
        assert!(!result.contains("Part 1"));
        assert!(!result.contains("Part 2"));
    }

    #[test]
    fn test_replace_text_escapes_special_chars() {
        let shape = r#"<p:sp><p:txBody><a:t>Old</a:t></p:txBody></p:sp>"#;
        let result = replace_text_in_shape_xml(shape, "A & B < C");
        assert!(result.contains("A &amp; B &lt; C"));
    }

    #[test]
    fn test_set_fill_replace_existing() {
        let shape =
            r#"<p:sp><p:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></p:spPr></p:sp>"#;
        let result = set_fill_in_shape_xml(shape, "#FF0000");
        assert!(result.contains(r#"val="FF0000""#));
        assert!(!result.contains("4472C4"));
    }

    #[test]
    fn test_set_fill_add_new() {
        let shape = r#"<p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/></a:xfrm></p:spPr></p:sp>"#;
        let result = set_fill_in_shape_xml(shape, "00FF00");
        assert!(result.contains(r#"<a:solidFill><a:srgbClr val="00FF00"/></a:solidFill>"#));
        // Should be inserted after <p:spPr>
        assert!(result.contains("<p:spPr><a:solidFill>"));
    }

    #[test]
    fn test_set_fill_strips_hash() {
        let shape = r#"<p:sp><p:spPr></p:spPr></p:sp>"#;
        let result = set_fill_in_shape_xml(shape, "#AABBCC");
        assert!(result.contains(r#"val="AABBCC""#));
        assert!(!result.contains("#"));
    }

    #[test]
    fn test_find_rid_for_slide() {
        let rels = r#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide2.xml"/>
  <Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide3.xml"/>
</Relationships>"#;

        assert_eq!(find_rid_for_slide(rels, 1).expect("should find"), "rId2");
        assert_eq!(find_rid_for_slide(rels, 3).expect("should find"), "rId4");
        assert!(find_rid_for_slide(rels, 99).is_err());
    }

    #[test]
    fn test_remove_slide_from_presentation_xml() {
        let xml = r#"<p:presentation>
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId2"/>
    <p:sldId id="257" r:id="rId3"/>
    <p:sldId id="258" r:id="rId4"/>
  </p:sldIdLst>
</p:presentation>"#;

        let result = remove_slide_from_presentation_xml(xml, "rId3").expect("should succeed");
        assert!(!result.contains("rId3"));
        assert!(result.contains("rId2"));
        assert!(result.contains("rId4"));
    }

    #[test]
    fn test_remove_slide_from_content_types() {
        let xml = r#"<Types>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/slides/slide2.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
</Types>"#;

        let result = remove_slide_from_content_types(xml, 1);
        assert!(!result.contains("slide1.xml"));
        assert!(result.contains("slide2.xml"));
    }

    #[test]
    fn test_remove_relationship() {
        let xml = r#"<Relationships>
  <Relationship Id="rId2" Type="slide" Target="slides/slide1.xml"/>
  <Relationship Id="rId3" Type="slide" Target="slides/slide2.xml"/>
</Relationships>"#;

        let result = remove_relationship(xml, "rId2");
        assert!(!result.contains("rId2"));
        assert!(result.contains("rId3"));
    }

    #[test]
    fn test_reorder_slides_move_forward() {
        let xml = r#"<p:presentation>
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId2"/>
    <p:sldId id="257" r:id="rId3"/>
    <p:sldId id="258" r:id="rId4"/>
  </p:sldIdLst>
</p:presentation>"#;

        // Move slide 1 to position 3
        let result = reorder_slides_in_presentation_xml(xml, 1, 3, 3).expect("should succeed");
        // After moving slide 1 to position 3: order should be rId3, rId4, rId2
        let r2 = result.find("rId2").expect("rId2 present");
        let r3 = result.find("rId3").expect("rId3 present");
        let r4 = result.find("rId4").expect("rId4 present");
        assert!(r3 < r4, "rId3 should come before rId4");
        assert!(r4 < r2, "rId4 should come before rId2");
    }

    #[test]
    fn test_reorder_slides_move_backward() {
        let xml = r#"<p:presentation>
  <p:sldIdLst>
    <p:sldId id="256" r:id="rId2"/>
    <p:sldId id="257" r:id="rId3"/>
    <p:sldId id="258" r:id="rId4"/>
  </p:sldIdLst>
</p:presentation>"#;

        // Move slide 3 to position 1
        let result = reorder_slides_in_presentation_xml(xml, 3, 1, 3).expect("should succeed");
        // After moving slide 3 to position 1: order should be rId4, rId2, rId3
        let r2 = result.find("rId2").expect("rId2 present");
        let r3 = result.find("rId3").expect("rId3 present");
        let r4 = result.find("rId4").expect("rId4 present");
        assert!(r4 < r2, "rId4 should come before rId2");
        assert!(r2 < r3, "rId2 should come before rId3");
    }
}
