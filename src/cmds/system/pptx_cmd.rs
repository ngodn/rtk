//! PowerPoint (.pptx) inspection and modification command.
//!
//! Uses the `pptx` crate for structured reading and slide management,
//! with regex-based XML manipulation for shape-level modifications
//! (set-text, set-fill) where the crate lacks direct write-back support.

use crate::core::tracking;
use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use pptx::enums::shapes::MsoAutoShapeType;
use pptx::{Emu, Presentation, Shape, ShapeTree};
use regex::Regex;
use std::path::Path;

// ── Reading helpers ────────────────────────────────────────────────────

/// Guess shape type from its name.
fn shape_type_label(name: &str) -> &str {
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

/// Extract text from a Shape (works for autoshapes with text frames).
fn shape_text(shape: &Shape) -> String {
    if let Some(auto) = shape.as_autoshape() {
        if let Some(tf) = auto.text_frame() {
            return tf.text();
        }
    }
    String::new()
}

/// Extract fill color hex from a Shape's autoshape fill, if any.
fn shape_fill_color(shape: &Shape) -> Option<String> {
    use pptx::{ColorFormat, FillFormat, SolidFill};
    if let Some(auto) = shape.as_autoshape() {
        if let Some(ref fill) = auto.fill {
            return match fill {
                FillFormat::Solid(SolidFill { color }) => match color {
                    ColorFormat::Rgb(rgb) => {
                        Some(format!("#{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b))
                    }
                    _ => Some(format!("{:?}", color)),
                },
                FillFormat::NoFill => None,
                other => Some(format!("{:?}", other)),
            };
        }
    }
    None
}

/// Extract font size (points) from first run of first paragraph.
fn shape_font_size(shape: &Shape) -> Option<f64> {
    if let Some(auto) = shape.as_autoshape() {
        if let Some(tf) = auto.text_frame() {
            for para in tf.paragraphs() {
                for run in para.runs() {
                    if let Some(sz) = run.font().size {
                        return Some(sz);
                    }
                }
            }
        }
    }
    None
}

/// Infer slide title from shapes (first shape whose name contains "title").
fn slide_title_from_tree(tree: &ShapeTree) -> String {
    // First try the pptx crate's title() method
    if let Some(title_shape) = tree.title() {
        let text = shape_text(title_shape);
        if !text.trim().is_empty() {
            return text.trim().to_string();
        }
    }
    // Fall back to name-based heuristic
    for shape in tree.iter() {
        if shape.name().to_lowercase().contains("title") {
            let text = shape_text(shape);
            if !text.trim().is_empty() {
                return text.trim().to_string();
            }
        }
    }
    // Fall back to first shape with text
    for shape in tree.iter() {
        let text = shape_text(shape);
        if !text.trim().is_empty() {
            return text.trim().to_string();
        }
    }
    "(untitled)".to_string()
}

/// Parse slide range: "3" -> (3, 3), "3-5" -> (3, 5).
fn parse_slide_range(spec: &str) -> Result<(usize, usize)> {
    if let Some((start_str, end_str)) = spec.split_once('-') {
        let start: usize = start_str
            .parse()
            .with_context(|| format!("Invalid slide number: {}", start_str))?;
        let end: usize = end_str
            .parse()
            .with_context(|| format!("Invalid slide number: {}", end_str))?;
        if start > end {
            bail!("Invalid range: {} > {}", start, end);
        }
        Ok((start, end))
    } else {
        let num: usize = spec
            .parse()
            .with_context(|| format!("Invalid slide number: {}", spec))?;
        Ok((num, num))
    }
}

// ── Subcommand implementations ─────────────────────────────────────────

/// `rtk pptx info <file>` - Show slide count, dimensions, metadata.
pub fn run_info(file: &Path, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs
        .slides()
        .with_context(|| format!("Failed to read slides from: {}", file.display()))?;
    let slide_count = slides.len();

    let mut output_lines: Vec<String> = Vec::new();
    output_lines.push(format!("Slides: {}", slide_count));

    // Metadata from core properties
    if let Ok(props) = prs.core_properties() {
        let title = props.title();
        if !title.is_empty() {
            output_lines.push(format!("Title: {}", title));
        }
        let author = props.author();
        if !author.is_empty() {
            output_lines.push(format!("Author: {}", author));
        }
        let modified_by = props.last_modified_by();
        if !modified_by.is_empty() {
            output_lines.push(format!("Last modified by: {}", modified_by));
        }
    }

    // Note: slide dimensions not directly exposed by pptx crate API

    let output = output_lines.join("\n");
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Slide count: {}", slide_count);
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

/// `rtk pptx slides <file>` - List all slides with titles.
pub fn run_slides(file: &Path, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs
        .slides()
        .with_context(|| format!("Failed to read slides from: {}", file.display()))?;

    if slides.is_empty() {
        bail!("No slides found in {}", file.display());
    }

    let mut output_lines: Vec<String> = Vec::new();

    for (i, slide_ref) in slides.iter().enumerate() {
        let title = match prs.slide_xml(slide_ref) {
            Ok(xml) => match ShapeTree::from_slide_xml(xml) {
                Ok(tree) => slide_title_from_tree(&tree),
                Err(_) => "(parse error)".to_string(),
            },
            Err(_) => "(read error)".to_string(),
        };
        output_lines.push(format!("{:>2}: {}", i + 1, title));
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
    let prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs
        .slides()
        .with_context(|| format!("Failed to read slides from: {}", file.display()))?;

    let (start, end) = parse_slide_range(slide_spec)?;

    let mut output_lines: Vec<String> = Vec::new();
    let mut raw_size_estimate = 0usize;

    for slide_num in start..=end {
        let idx = slide_num
            .checked_sub(1)
            .context("Slide number must be >= 1")?;

        if idx >= slides.len() {
            output_lines.push(format!("Slide {}: (not found)", slide_num));
            continue;
        }

        let slide_ref = &slides[idx];
        let xml = prs
            .slide_xml(slide_ref)
            .with_context(|| format!("Failed to read slide {}", slide_num))?;
        raw_size_estimate += xml.len();

        let tree = ShapeTree::from_slide_xml(xml).unwrap_or_else(|_| {
            // Return an empty tree on parse error
            ShapeTree::from_slide_xml(b"<p:sld><p:cSld><p:spTree></p:spTree></p:cSld></p:sld>")
                .unwrap_or_else(|_| unreachable!())
        });
        let title = slide_title_from_tree(&tree);

        if !output_lines.is_empty() {
            output_lines.push(String::new());
        }
        output_lines.push(format!("Slide {}: {}", slide_num, title));
        output_lines.push(String::new());

        for shape in tree.iter() {
            let stype = shape_type_label(shape.name());
            let text = shape_text(shape);
            let mut desc = format!("[{}]", stype);

            if !text.trim().is_empty() {
                desc.push_str(&format!(" \"{}\"", text.trim()));
            }

            if let Some(sz) = shape_font_size(shape) {
                if sz > 0.0 {
                    desc.push_str(&format!(" ({:.0}pt)", sz));
                }
            }

            if let Some(ref fill) = shape_fill_color(shape) {
                desc.push_str(&format!(" fill:{}", fill));
            }

            // Skip shapes with no useful content
            if text.trim().is_empty() && shape_fill_color(shape).is_none() {
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
    let prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs
        .slides()
        .with_context(|| format!("Failed to read slides from: {}", file.display()))?;

    let query_lower = query.to_lowercase();
    let mut output_lines: Vec<String> = Vec::new();
    let mut raw_size_estimate = 0usize;

    for (i, slide_ref) in slides.iter().enumerate() {
        let slide_num = i + 1;
        let xml = match prs.slide_xml(slide_ref) {
            Ok(xml) => xml,
            Err(_) => continue,
        };
        raw_size_estimate += xml.len();

        let tree = match ShapeTree::from_slide_xml(xml) {
            Ok(tree) => tree,
            Err(_) => continue,
        };

        for (j, shape) in tree.iter().enumerate() {
            let text = shape_text(shape);
            if text.to_lowercase().contains(&query_lower) {
                let text_preview = if text.len() > 80 {
                    let truncated: String = text.chars().take(77).collect();
                    format!("{}...", truncated)
                } else {
                    text.clone()
                };
                output_lines.push(format!(
                    "Slide {}, Shape {}: \"{}\"",
                    slide_num,
                    j + 1,
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

// ── Write helpers (regex-based XML manipulation) ───────────────────────

lazy_static! {
    /// Match a cNvPr element with a specific name attribute.
    static ref CNVPR_NAME_RE: Regex =
        Regex::new(r#"<[^>]*cNvPr[^>]*\bname="([^"]*)"[^>]*/?\s*>"#).expect("valid regex");
}

/// Find the XML range of a shape with the given name in slide XML.
/// Returns the byte range of the `<p:sp>...</p:sp>` block and the shape's XML.
fn find_shape_xml_by_name<'a>(xml: &'a str, shape_name: &str) -> Option<(usize, usize, &'a str)> {
    let sp_open_tag = "<p:sp>";
    let sp_open_tag_with_attrs = "<p:sp ";
    let sp_close_tag = "</p:sp>";

    let mut search_from = 0;
    loop {
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

    lazy_static! {
        static ref SOLID_FILL_RE: Regex =
            Regex::new(r"<a:solidFill>.*?</a:solidFill>").expect("valid regex");
        static ref SPPR_OPEN_RE: Regex = Regex::new(r"<p:spPr[^>]*>").expect("valid regex");
    }

    if SOLID_FILL_RE.is_match(shape_xml) {
        return SOLID_FILL_RE
            .replace(shape_xml, fill_xml.as_str())
            .to_string();
    }

    if let Some(m) = SPPR_OPEN_RE.find(shape_xml) {
        let insert_pos = m.end();
        let mut result = String::with_capacity(shape_xml.len() + fill_xml.len());
        result.push_str(&shape_xml[..insert_pos]);
        result.push_str(&fill_xml);
        result.push_str(&shape_xml[insert_pos..]);
        return result;
    }

    shape_xml.to_string()
}

/// Helper: apply a transformation to a specific slide's XML via the pptx crate.
fn transform_slide_xml<F>(prs: &mut Presentation, slide_idx: usize, transform: F) -> Result<()>
where
    F: FnOnce(&str) -> Result<String>,
{
    let slides = prs.slides().context("Failed to read slides")?;
    if slide_idx >= slides.len() {
        bail!(
            "Slide {} out of range (has {} slides)",
            slide_idx + 1,
            slides.len()
        );
    }
    let slide_ref = slides[slide_idx].clone();
    let xml_bytes = prs
        .slide_xml(&slide_ref)
        .with_context(|| format!("Failed to read slide {}", slide_idx + 1))?;
    let xml_str = std::str::from_utf8(xml_bytes).context("Invalid UTF-8 in slide XML")?;
    let new_xml = transform(xml_str)?;
    let dest = prs.slide_xml_mut(&slide_ref).with_context(|| {
        format!(
            "Failed to get mutable slide XML for slide {}",
            slide_idx + 1
        )
    })?;
    *dest = new_xml.into_bytes();
    Ok(())
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
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let shape_name_owned = shape_name.to_string();
    let new_text_owned = new_text.to_string();

    transform_slide_xml(&mut prs, (slide as usize) - 1, |xml| {
        let (start, end, shape_block) = find_shape_xml_by_name(xml, &shape_name_owned)
            .with_context(|| {
                format!("Shape '{}' not found on slide {}", shape_name_owned, slide)
            })?;

        let new_block = replace_text_in_shape_xml(shape_block, &new_text_owned);
        let mut result = String::with_capacity(xml.len());
        result.push_str(&xml[..start]);
        result.push_str(&new_block);
        result.push_str(&xml[end..]);
        Ok(result)
    })?;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

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
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let shape_name_owned = shape_name.to_string();
    let hex_color_owned = hex_color.to_string();

    transform_slide_xml(&mut prs, (slide as usize) - 1, |xml| {
        let (start, end, shape_block) = find_shape_xml_by_name(xml, &shape_name_owned)
            .with_context(|| {
                format!("Shape '{}' not found on slide {}", shape_name_owned, slide)
            })?;

        let new_block = set_fill_in_shape_xml(shape_block, &hex_color_owned);
        let mut result = String::with_capacity(xml.len());
        result.push_str(&xml[..start]);
        result.push_str(&new_block);
        result.push_str(&xml[end..]);
        Ok(result)
    })?;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

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
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs.slides().context("Failed to read slides")?;
    let idx = (slide as usize)
        .checked_sub(1)
        .context("Slide number must be >= 1")?;

    if idx >= slides.len() {
        bail!(
            "Slide {} not found in {} (has {} slides)",
            slide,
            file.display(),
            slides.len()
        );
    }

    if slides.len() <= 1 {
        bail!("Cannot delete the only slide in the presentation");
    }

    let slide_ref = slides[idx].clone();
    prs.delete_slide(&slide_ref)
        .with_context(|| format!("Failed to delete slide {}", slide))?;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

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

/// `rtk pptx move-slide <file> <from> <to>`
pub fn run_move_slide(file: &Path, from: u32, to: u32, verbose: u8) -> Result<()> {
    let timer = tracking::TimedExecution::start();

    if from == to {
        println!("Slide {} is already at position {}", from, to);
        return Ok(());
    }

    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs.slides().context("Failed to read slides")?;
    let count = slides.len();

    if from < 1 || (from as usize) > count {
        bail!("Slide {} out of range (1-{})", from, count);
    }
    if to < 1 || (to as usize) > count {
        bail!("Destination {} out of range (1-{})", to, count);
    }

    drop(slides);

    prs.move_slide((from as usize) - 1, (to as usize) - 1)
        .with_context(|| format!("Failed to move slide {} to position {}", from, to))?;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

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

// ── New subcommand implementations (using pptx crate shape addition) ──

/// `rtk pptx add-textbox <file> <slide> --left --top --width --height [--text]`
pub fn run_add_textbox(
    file: &Path,
    slide: usize,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    text: Option<&str>,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs.slides().context("Failed to read slides")?;
    let idx = slide.checked_sub(1).context("Slide number must be >= 1")?;
    if idx >= slides.len() {
        bail!("Slide {} not found (has {} slides)", slide, slides.len());
    }
    let slide_ref = slides[idx].clone();

    let xml = prs
        .slide_xml(&slide_ref)
        .context("Failed to read slide XML")?
        .to_vec();

    let new_xml = ShapeTree::add_textbox(&xml, Emu(left), Emu(top), Emu(width), Emu(height))
        .context("Failed to add textbox to slide XML")?;

    // If text was provided, find the newly added textbox and set its text
    let final_xml = if let Some(txt) = text {
        // The new textbox is the last <p:sp> in the XML. Set its <a:t> content.
        set_last_shape_text(&new_xml, txt)
    } else {
        new_xml
    };

    let dest = prs
        .slide_xml_mut(&slide_ref)
        .context("Failed to get mutable slide XML")?;
    *dest = final_xml;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

    let output = format!(
        "Added textbox to slide {} at ({}, {}) size {}x{}{}",
        slide,
        left,
        top,
        width,
        height,
        text.map(|t| format!(" with text \"{}\"", t))
            .unwrap_or_default()
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx add-textbox {}", file.display()),
        "rtk pptx add-textbox",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// `rtk pptx add-shape <file> <slide> <shape-type> --left --top --width --height [--fill] [--text]`
pub fn run_add_shape(
    file: &Path,
    slide: usize,
    shape_type: &str,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    fill: Option<&str>,
    text: Option<&str>,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs.slides().context("Failed to read slides")?;
    let idx = slide.checked_sub(1).context("Slide number must be >= 1")?;
    if idx >= slides.len() {
        bail!("Slide {} not found (has {} slides)", slide, slides.len());
    }
    let slide_ref = slides[idx].clone();

    let xml = prs
        .slide_xml(&slide_ref)
        .context("Failed to read slide XML")?
        .to_vec();

    let auto_shape_type = parse_shape_type(shape_type)?;

    let new_xml = ShapeTree::add_shape(
        &xml,
        auto_shape_type,
        Emu(left),
        Emu(top),
        Emu(width),
        Emu(height),
    )
    .context("Failed to add shape to slide XML")?;

    // Apply optional fill and text to the last shape
    let mut final_xml = new_xml;

    if let Some(color) = fill {
        final_xml = set_last_shape_fill(&final_xml, color);
    }

    if let Some(txt) = text {
        final_xml = set_last_shape_text(&final_xml, txt);
    }

    let dest = prs
        .slide_xml_mut(&slide_ref)
        .context("Failed to get mutable slide XML")?;
    *dest = final_xml;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

    let output = format!(
        "Added {} to slide {} at ({}, {}) size {}x{}",
        shape_type, slide, left, top, width, height
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx add-shape {}", file.display()),
        "rtk pptx add-shape",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

/// `rtk pptx add-table <file> <slide> <rows> <cols> --left --top --width --height`
pub fn run_add_table(
    file: &Path,
    slide: usize,
    rows: u32,
    cols: u32,
    left: i64,
    top: i64,
    width: i64,
    height: i64,
    verbose: u8,
) -> Result<()> {
    let timer = tracking::TimedExecution::start();
    let mut prs = Presentation::open(file)
        .with_context(|| format!("Failed to open PPTX: {}", file.display()))?;

    let slides = prs.slides().context("Failed to read slides")?;
    let idx = slide.checked_sub(1).context("Slide number must be >= 1")?;
    if idx >= slides.len() {
        bail!("Slide {} not found (has {} slides)", slide, slides.len());
    }
    let slide_ref = slides[idx].clone();

    let xml = prs
        .slide_xml(&slide_ref)
        .context("Failed to read slide XML")?
        .to_vec();

    let new_xml = ShapeTree::add_table(
        &xml,
        rows,
        cols,
        Emu(left),
        Emu(top),
        Emu(width),
        Emu(height),
    )
    .context("Failed to add table to slide XML")?;

    let dest = prs
        .slide_xml_mut(&slide_ref)
        .context("Failed to get mutable slide XML")?;
    *dest = new_xml;

    prs.save(file)
        .with_context(|| format!("Failed to save PPTX: {}", file.display()))?;

    let output = format!(
        "Added {}x{} table to slide {} at ({}, {}) size {}x{}",
        rows, cols, slide, left, top, width, height
    );
    println!("{}", output);

    if verbose > 0 {
        eprintln!("Modified: {}", file.display());
    }

    timer.track(
        &format!("pptx add-table {}", file.display()),
        "rtk pptx add-table",
        "manual edit in PowerPoint",
        &output,
    );
    Ok(())
}

// ── Helpers for new subcommands ────────────────────────────────────────

/// Parse a shape type string into the pptx crate's MsoAutoShapeType.
fn parse_shape_type(s: &str) -> Result<MsoAutoShapeType> {
    match s.to_lowercase().as_str() {
        "rectangle" | "rect" => Ok(MsoAutoShapeType::Rectangle),
        "rounded-rectangle" | "rounded-rect" | "roundrect" => {
            Ok(MsoAutoShapeType::RoundedRectangle)
        }
        "oval" | "ellipse" => Ok(MsoAutoShapeType::Oval),
        "diamond" => Ok(MsoAutoShapeType::Diamond),
        "triangle" => Ok(MsoAutoShapeType::IsoscelesTriangle),
        "right-triangle" => Ok(MsoAutoShapeType::RightTriangle),
        "pentagon" => Ok(MsoAutoShapeType::Pentagon),
        "hexagon" => Ok(MsoAutoShapeType::Hexagon),
        "octagon" => Ok(MsoAutoShapeType::Octagon),
        "star5" | "star" => Ok(MsoAutoShapeType::Star5Point),
        "star4" => Ok(MsoAutoShapeType::Star4Point),
        "arrow" | "right-arrow" => Ok(MsoAutoShapeType::RightArrow),
        "left-arrow" => Ok(MsoAutoShapeType::LeftArrow),
        "up-arrow" => Ok(MsoAutoShapeType::UpArrow),
        "down-arrow" => Ok(MsoAutoShapeType::DownArrow),
        "chevron" => Ok(MsoAutoShapeType::Chevron),
        "cloud" => Ok(MsoAutoShapeType::Cloud),
        "heart" => Ok(MsoAutoShapeType::Heart),
        "lightning-bolt" | "lightning" => Ok(MsoAutoShapeType::LightningBolt),
        "sun" => Ok(MsoAutoShapeType::Sun),
        "moon" => Ok(MsoAutoShapeType::Moon),
        "plus" | "cross" => Ok(MsoAutoShapeType::Cross),
        _ => bail!(
            "Unknown shape type '{}'. Supported: rectangle, rounded-rectangle, oval, diamond, \
             triangle, pentagon, hexagon, octagon, star, arrow, chevron, cloud, heart, \
             lightning, sun, moon, plus",
            s
        ),
    }
}

/// Set text on the last <p:sp> in XML bytes (for newly added shapes).
fn set_last_shape_text(xml_bytes: &[u8], text: &str) -> Vec<u8> {
    let xml_str = match std::str::from_utf8(xml_bytes) {
        Ok(s) => s,
        Err(_) => return xml_bytes.to_vec(),
    };

    // Find the last </p:sp> and work backwards to find its <p:sp>
    if let Some(last_close) = xml_str.rfind("</p:sp>") {
        let before = &xml_str[..last_close];
        // Find the matching opening tag
        if let Some(last_open) = before.rfind("<p:sp>").or_else(|| before.rfind("<p:sp ")) {
            let end = last_close + "</p:sp>".len();
            let shape_block = &xml_str[last_open..end];
            let new_block = replace_text_in_shape_xml(shape_block, text);
            let mut result = String::with_capacity(xml_str.len());
            result.push_str(&xml_str[..last_open]);
            result.push_str(&new_block);
            result.push_str(&xml_str[end..]);
            return result.into_bytes();
        }
    }

    xml_bytes.to_vec()
}

/// Set fill color on the last <p:sp> in XML bytes (for newly added shapes).
fn set_last_shape_fill(xml_bytes: &[u8], hex_color: &str) -> Vec<u8> {
    let xml_str = match std::str::from_utf8(xml_bytes) {
        Ok(s) => s,
        Err(_) => return xml_bytes.to_vec(),
    };

    if let Some(last_close) = xml_str.rfind("</p:sp>") {
        let before = &xml_str[..last_close];
        if let Some(last_open) = before.rfind("<p:sp>").or_else(|| before.rfind("<p:sp ")) {
            let end = last_close + "</p:sp>".len();
            let shape_block = &xml_str[last_open..end];
            let new_block = set_fill_in_shape_xml(shape_block, hex_color);
            let mut result = String::with_capacity(xml_str.len());
            result.push_str(&xml_str[..last_open]);
            result.push_str(&new_block);
            result.push_str(&xml_str[end..]);
            return result.into_bytes();
        }
    }

    xml_bytes.to_vec()
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
    fn test_shape_type_detection() {
        assert_eq!(shape_type_label("Title 1"), "TextBox");
        assert_eq!(shape_type_label("Rectangle 5"), "Rectangle");
        assert_eq!(shape_type_label("Content Placeholder 1"), "TextBox");
        assert_eq!(shape_type_label("Arrow Connector 3"), "Arrow");
        assert_eq!(shape_type_label("Freeform 7"), "Shape");
    }

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
        assert!(result.contains(r#"sz="2400""#));
    }

    #[test]
    fn test_replace_text_multiple_runs() {
        let shape = r#"<p:sp><p:txBody><a:p><a:r><a:t>Part 1</a:t></a:r><a:r><a:t>Part 2</a:t></a:r></a:p></p:txBody></p:sp>"#;
        let result = replace_text_in_shape_xml(shape, "Full Replacement");
        assert!(result.contains("<a:t>Full Replacement</a:t>"));
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
    fn test_parse_shape_type_valid() {
        assert!(parse_shape_type("rectangle").is_ok());
        assert!(parse_shape_type("oval").is_ok());
        assert!(parse_shape_type("arrow").is_ok());
        assert!(parse_shape_type("diamond").is_ok());
    }

    #[test]
    fn test_parse_shape_type_invalid() {
        assert!(parse_shape_type("nonexistent-shape").is_err());
    }
}
