# Rebuild rtk pptx Using the pptx Rust Crate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the raw zip+xml pptx implementation with the proper `pptx` Rust crate. This unlocks full write support: add textboxes, add shapes, add tables, modify text with font/color, modify fills, add images.

**Architecture:** Replace `zip` + `quick-xml` manual parsing with `pptx` crate's `Presentation::open()` + `ShapeTree::from_slide_xml()` + `ShapeTree::add_*()` methods. Keep existing subcommand interface, add new ones.

**Tech Stack:** pptx crate v0.1.0

---

## Task 1: Replace zip dependency with pptx crate

**Files:**
- Modify: `Cargo.toml` (replace `zip = "2"` with `pptx = "0.1"`)
- Modify: `src/cmds/system/pptx_cmd.rs` (rewrite using pptx crate)

Steps:
- [ ] Remove `zip = "2"` from Cargo.toml, add `pptx = "0.1"`
- [ ] Rewrite `run_info` using `Presentation::open()`, `prs.slide_count()`, `prs.core_properties()`, `prs.slide_size()`
- [ ] Rewrite `run_slides` using `prs.slides()`, `ShapeTree::from_slide_xml()`, `tree.title()`
- [ ] Rewrite `run_read` using `ShapeTree::from_slide_xml()`, iterate `tree.shapes`, extract text via `as_autoshape().text_frame().text()`, fill via `shape.fill`, font via runs
- [ ] Rewrite `run_find` using same pattern, search across all slides
- [ ] Keep `run_set_text`, `run_set_fill`, `run_delete_slide`, `run_move_slide` but rewrite using pptx crate's `prs.slide_xml_mut()`, `prs.delete_slide()`, `prs.move_slide()`
- [ ] Run tests, commit

## Task 2: Add new write subcommands

**Files:**
- Modify: `src/main.rs` (add new PptxCommands variants)
- Modify: `src/cmds/system/pptx_cmd.rs` (add implementations)

New subcommands:
- [ ] `rtk pptx add-textbox <file> <slide> --left <emu> --top <emu> --width <emu> --height <emu> --text "content" [--font-size 24] [--bold] [--color "#FF0000"]`
  Uses `ShapeTree::add_textbox()` then modifies the XML to set text/font
- [ ] `rtk pptx add-shape <file> <slide> <shape-type> --left --top --width --height [--fill "#color"] [--text "label"]`
  Uses `ShapeTree::add_shape()` with `MsoAutoShapeType`
- [ ] `rtk pptx add-table <file> <slide> <rows> <cols> --left --top --width --height`
  Uses `ShapeTree::add_table()`
- [ ] `rtk pptx set-font <file> <slide> <shape-name> [--size 24] [--bold] [--italic] [--color "#FF0000"] [--name "Calibri"]`
  Parse shape, modify font via `run.font_mut()`
- [ ] Run tests, commit

## Task 3: Update superpowers skill

**Files:**
- Modify: `~/.claude/skills/pptx-editing/SKILL.md`
- Modify: `/home/eins0fx/development/claude-code/superpowers/skills/pptx-editing/SKILL.md`

Update the skill to reflect all actual RTK pptx capabilities based on what's implemented. Remove python-pptx examples for operations RTK now handles. Keep python-pptx fallback only for things RTK truly can't do (animations, SmartArt editing, complex group shape manipulation).
