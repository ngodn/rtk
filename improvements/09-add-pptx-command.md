# Add `rtk pptx` Command

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `rtk pptx` command that reads and inspects PowerPoint files fast in Rust, providing token-optimized output for LLM consumption. Supports: info, slides list, read slide details, find text.

**Architecture:** New `src/cmds/system/pptx_cmd.rs` module using the `pptx` Rust crate for OOXML parsing. Subcommands: `info`, `slides`, `read`, `find`. All output is compact and token-optimized.

**Tech Stack:** Rust, `pptx` crate (pure Rust, zip + quick-xml), clap subcommands

---

### Task 1: Add pptx dependency and create module skeleton

**Files:**
- Modify: `Cargo.toml` (add pptx dependency)
- Create: `src/cmds/system/pptx_cmd.rs`
- Modify: `src/cmds/system/mod.rs` (add module)
- Modify: `src/main.rs` (add Pptx command variant and routing)

- [ ] **Step 1: Add the pptx crate dependency**

Check crates.io for the latest version of the `pptx` crate. Add it to Cargo.toml dependencies.

If the `pptx` crate cannot parse existing files well, fall back to raw zip + quick-xml approach (PPTX is just a zip of XML files). RTK already has `quick-xml` as a dependency.

- [ ] **Step 2: Create the command module skeleton**

Create `src/cmds/system/pptx_cmd.rs` with four public functions:

```rust
pub fn run_info(file: &Path, verbose: u8) -> Result<()>
pub fn run_slides(file: &Path, verbose: u8) -> Result<()>
pub fn run_read(file: &Path, slide_spec: &str, verbose: u8) -> Result<()>
pub fn run_find(file: &Path, query: &str, verbose: u8) -> Result<()>
```

- [ ] **Step 3: Add Pptx command to main.rs Commands enum**

```rust
/// PowerPoint file inspection and editing
Pptx {
    #[command(subcommand)]
    command: PptxCommands,
},
```

With subcommands:
```rust
enum PptxCommands {
    /// Show presentation info (slide count, dimensions, metadata)
    Info { file: PathBuf },
    /// List all slides with titles
    Slides { file: PathBuf },
    /// Read slide details (shapes, text, positions)
    Read { file: PathBuf, /// Slide number or range (e.g., "3" or "3-5")
           slide: String },
    /// Find shapes containing text
    Find { file: PathBuf, query: String },
}
```

- [ ] **Step 4: Route in main.rs**

- [ ] **Step 5: Add module to mod.rs**

- [ ] **Step 6: Verify compiles**

```bash
cargo build
```

- [ ] **Step 7: Commit skeleton**

```bash
git commit -m "feat(pptx): add pptx command skeleton with info/slides/read/find subcommands"
```

---

### Task 2: Implement PPTX reading using zip + quick-xml

Since PPTX is just a zip file containing XML, and RTK already has both `zip` (via other deps) and `quick-xml`, we can parse it directly without a heavy dependency.

**Files:**
- Modify: `src/cmds/system/pptx_cmd.rs`

- [ ] **Step 1: Implement zip-based PPTX reader**

PPTX structure:
```
presentation.pptx (zip)
  [Content_Types].xml
  ppt/presentation.xml     <- slide references
  ppt/slides/slide1.xml    <- slide content
  ppt/slides/slide2.xml
  ppt/slideMasters/
  ppt/slideLayouts/
  docProps/app.xml          <- metadata
  docProps/core.xml         <- metadata
```

Parse `ppt/presentation.xml` for slide list, then each `ppt/slides/slideN.xml` for content.

- [ ] **Step 2: Implement `run_info`**

Output format:
```
Slides: 22
Dimensions: 13333600x7559675 EMU (33.87cm x 19.05cm)
Title: Mediacorp ITAM
Author: (if available)
```

- [ ] **Step 3: Implement `run_slides`**

Output format:
```
 1: Title Slide - Mediacorp ITAM
 2: Agenda
 3: Asset Lifecycle Overview
 4: New Asset Request Workflow
...
22: Q&A
```

Extract title from the first text shape on each slide.

- [ ] **Step 4: Implement `run_read`**

Output format for `rtk pptx read file.pptx 3`:
```
Slide 3: Asset Lifecycle Overview

Shapes (8):
  [1] TextBox "Asset Lifecycle" at (500,200) 8000x600
      Text: "Asset Lifecycle Overview"
      Font: Calibri 24pt Bold
  [2] Rectangle "Ordered" at (1000,1500) 2000x800
      Text: "Ordered"
      Fill: #4472C4
  [3] Arrow at (3000,1900) 500x50
  ...
```

- [ ] **Step 5: Implement `run_find`**

Output format for `rtk pptx find file.pptx "Ordered"`:
```
Slide 3, Shape 2: Rectangle "Ordered"
  Text: "Ordered"
Slide 7, Shape 5: TextBox
  Text: "Status: Ordered -> In Progress"
```

- [ ] **Step 6: Add token tracking**

Track input (raw XML size) vs output (compact summary) for RTK gain reporting.

- [ ] **Step 7: Run tests and commit**

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test --all
git commit -m "feat(pptx): implement info/slides/read/find using zip + quick-xml"
```

---

### Task 3: Test with real PPTX file

- [ ] **Step 1: Test against the Mediacorp ITAM presentation**

```bash
rtk pptx info /home/eins0fx/development/izeno/mediacorp/itam/Mediacorp-ITAM.pptx
rtk pptx slides /home/eins0fx/development/izeno/mediacorp/itam/Mediacorp-ITAM.pptx
rtk pptx read /home/eins0fx/development/izeno/mediacorp/itam/Mediacorp-ITAM.pptx 3
rtk pptx find /home/eins0fx/development/izeno/mediacorp/itam/Mediacorp-ITAM.pptx "Asset"
```

- [ ] **Step 2: Verify output is useful and compact**

The output should give Claude enough context to understand the slide structure without reading the entire raw XML.

- [ ] **Step 3: Install and verify**

```bash
cargo install --path .
rtk pptx info /path/to/any.pptx
```
