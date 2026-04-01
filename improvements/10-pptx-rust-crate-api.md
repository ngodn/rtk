# pptx Rust Crate: Complete API Reference

Source: https://github.com/hidemi-ito/rust-pptx (38,410 lines)
Crate: https://crates.io/crates/pptx v0.1.0

## Presentation (core entry point)

```rust
// Lifecycle
Presentation::new() -> PptxResult<Self>
Presentation::open(path) -> PptxResult<Self>
Presentation::from_bytes(data: &[u8]) -> PptxResult<Self>
Presentation::from_reader(reader) -> PptxResult<Self>
prs.save(path) -> PptxResult<()>
prs.to_bytes() -> PptxResult<Vec<u8>>
prs.write_to(writer) -> PptxResult<()>

// Slides
prs.slides() -> PptxResult<Vec<SlideRef>>
prs.slide_count() -> PptxResult<usize>
prs.slides_get(index: usize) -> PptxResult<SlideRef>
prs.slide_index(&slide_ref) -> PptxResult<usize>
prs.add_slide(&layout) -> PptxResult<SlideRef>
prs.delete_slide(&slide_ref) -> PptxResult<()>
prs.move_slide(from, to) -> PptxResult<()>
prs.slide_xml(&slide_ref) -> PptxResult<&[u8]>
prs.slide_xml_mut(&slide_ref) -> PptxResult<&mut Vec<u8>>
prs.slide_name(&slide_ref) -> PptxResult<Option<String>>
prs.slide_size() -> PptxResult<Option<(i64, i64)>>

// Layouts
prs.slide_layouts() -> PptxResult<Vec<SlideLayoutRef>>

// Media
prs.add_image(&image) -> PptxResult<String>  // returns rId
prs.add_chart_to_slide(&slide, &data, chart_type, left, top, width, height)
prs.add_video_to_slide(&slide, &video, &poster, left, top, width, height)

// Properties
prs.core_properties() -> PptxResult<CoreProperties>
prs.set_core_properties(&props) -> PptxResult<()>
```

## ShapeTree (shape container per slide)

```rust
// Parse from slide XML
ShapeTree::from_slide_xml(xml: &[u8]) -> PptxResult<Self>

// Access
tree.shapes: Vec<Shape>          // public field
tree.iter() -> impl Iterator<Item = &Shape>
tree.len() -> usize
tree.is_empty() -> bool
tree.max_shape_id() -> ShapeId
tree.title() -> Option<&Shape>
tree.placeholders() -> Vec<&Shape>

// Add shapes (static methods, take slide XML, return updated XML)
ShapeTree::add_shape(slide_xml, MsoAutoShapeType, left, top, width, height) -> PptxResult<Vec<u8>>
ShapeTree::add_textbox(slide_xml, left, top, width, height) -> PptxResult<Vec<u8>>
ShapeTree::add_picture(slide_xml, image_r_id, left, top, width, height) -> PptxResult<Vec<u8>>
ShapeTree::add_table(slide_xml, rows, cols, left, top, width, height) -> PptxResult<Vec<u8>>
ShapeTree::add_connector(slide_xml, MsoConnectorType, begin_x, begin_y, end_x, end_y) -> PptxResult<Vec<u8>>
ShapeTree::add_group_shape(slide_xml, left, top, width, height) -> PptxResult<Vec<u8>>
ShapeTree::add_movie(slide_xml, video_r_id, poster_r_id, left, top, width, height) -> PptxResult<Vec<u8>>

// XML generation helpers
ShapeTree::new_textbox_xml(id, name, left, top, width, height) -> String
ShapeTree::new_autoshape_xml(id, name, left, top, width, height, prst) -> String
ShapeTree::new_picture_xml(id, name, desc, r_id, left, top, width, height) -> String
ShapeTree::new_table_xml(id, name, rows, cols, left, top, width, height) -> String
ShapeTree::new_connector_xml(id, name, left, top, width, height, prst) -> String
```

## Shape (enum with accessors)

```rust
pub enum Shape {
    AutoShape(Box<AutoShape>),
    Picture(Box<Picture>),
    GraphicFrame(Box<GraphicFrame>),
    Connector(Connector),
    GroupShape(Box<GroupShape>),
    OleObject(OleObject),
}

// Common via ShapeProperties trait
shape.shape_id() -> ShapeId
shape.name() -> &str
shape.left() -> Emu
shape.top() -> Emu
shape.width() -> Emu
shape.height() -> Emu
shape.rotation() -> f64
shape.has_text_frame() -> bool
shape.has_table() -> bool
shape.is_placeholder() -> bool

// Type-specific accessors
shape.as_autoshape() -> Option<&AutoShape>
shape.as_autoshape_mut() -> Option<&mut AutoShape>
shape.as_picture() -> Option<&Picture>
shape.as_graphic_frame() -> Option<&GraphicFrame>
shape.as_group() -> Option<&GroupShape>
shape.as_connector() -> Option<&Connector>
```

## AutoShape

```rust
// Constructors
AutoShape::new(shape_id, name, left, top, width, height)
AutoShape::textbox(shape_id, name, left, top, width, height)
AutoShape::with_geometry(shape_id, name, left, top, width, height, PresetGeometry)

// Methods
shape.has_text_frame() -> bool
shape.text_frame() -> Option<&TextFrame>
shape.text_frame_mut() -> Option<&mut TextFrame>
shape.set_fill(FillFormat)
shape.set_line(LineFormat)
shape.set_click_action(ActionSetting)
shape.set_shadow(ShadowFormat)
shape.set_scene_3d(Scene3D)
shape.set_shape_3d(Shape3D)

// Public fields
shape.fill: Option<FillFormat>
shape.line: Option<LineFormat>
shape.prst_geom: Option<PresetGeometry>
shape.is_textbox: bool
```

## TextFrame / Paragraph / Run / Font

```rust
// TextFrame
tf = TextFrame::new()
tf.text() -> String
tf.set_text("Hello")
tf.paragraphs() -> &[Paragraph]
tf.paragraphs_mut() -> &mut [Paragraph]
tf.add_paragraph() -> &mut Paragraph
tf.clear()
tf.word_wrap: bool
tf.auto_size: MsoAutoSize
tf.rotation: Option<f64>

// Paragraph
p = Paragraph::new()
p.text() -> String
p.runs() -> &[Run]
p.runs_mut() -> &mut [Run]
p.add_run() -> &mut Run
p.add_line_break() -> &mut Run
p.set_alignment(PpParagraphAlignment::Center)
p.set_bullet(BulletFormat::Character('•'))
p.bullet_color: Option<ColorFormat>
p.font: Option<Font>
p.clear()

// Run
r = Run::new()
r.text() -> &str
r.set_text("Hello")
r.font() -> &Font
r.font_mut() -> &mut Font
r.set_hyperlink(Hyperlink)
r.set_language("en-US")

// Font
f = Font::new()
f.name: Option<String>
f.size: Option<f64>           // points
f.bold: Option<bool>
f.italic: Option<bool>
f.underline: Option<MsoTextUnderlineType>
f.color: Option<RgbColor>
f.strikethrough: Option<bool>
f.set_size(24.0) -> Result<(), PptxError>
```

## Table / Cell

```rust
// Table
Table::new(rows, cols, total_width: Emu, row_height: Emu)
table.cell(row, col) -> &Cell
table.cell_mut(row, col) -> &mut Cell
table.row_count() -> usize
table.col_count() -> usize
table.add_row() -> &mut Row
table.add_column(width: Emu)

// Cell
cell.text() -> String
cell.set_text("Hello")
cell.text_frame: TextFrame     // full access to rich text
cell.fill: Option<FillFormat>
cell.borders: CellBorders
cell.merge_with(span_width, span_height)
```

## Colors / Fills / Lines

```rust
// RgbColor
RgbColor::new(r, g, b)
RgbColor::from_hex("#FF0000") -> Result<Self, PptxError>
color.to_hex() -> String

// ColorFormat
ColorFormat::rgb(r, g, b)
ColorFormat::theme(MsoThemeColorIndex)
ColorFormat::hsl(hue, sat, lum)

// FillFormat
FillFormat::solid(ColorFormat)
FillFormat::no_fill()
FillFormat::linear_gradient(start_color, end_color, angle)

// LineFormat
LineFormat::new()
LineFormat::solid(ColorFormat, width: Emu)
```

## Round-Trip Pattern

```rust
// 1. Open
let mut prs = Presentation::open("file.pptx")?;
let slide_ref = prs.slides_get(14)?;

// 2. Parse shapes from slide XML
let xml = prs.slide_xml(&slide_ref)?;
let tree = ShapeTree::from_slide_xml(xml)?;

// 3. Read shapes
for shape in tree.iter() {
    if let Some(auto) = shape.as_autoshape() {
        if let Some(tf) = auto.text_frame() {
            println!("{}: {}", auto.name, tf.text());
        }
    }
}

// 4. Add new shape (returns updated XML)
let new_xml = ShapeTree::add_textbox(xml, left, top, width, height)?;
*prs.slide_xml_mut(&slide_ref)? = new_xml;

// 5. Save
prs.save("file.pptx")?;
```

## Parsers (for modifying existing shapes)

```rust
// Parse existing shape properties from XML fragments
parse_text_frame_from_xml(tx_body_bytes) -> PptxResult<Option<TextFrame>>
parse_fill_from_xml(fill_bytes) -> PptxResult<Option<FillFormat>>
parse_line_from_xml(line_bytes) -> PptxResult<Option<LineFormat>>
parse_sp_pr(sp_pr_bytes) -> PptxResult<(Option<FillFormat>, Option<LineFormat>)>
```
