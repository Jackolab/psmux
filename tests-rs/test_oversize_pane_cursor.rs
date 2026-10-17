//! Regression guard for the `window_bigger` caret (tenax#2033): when the
//! pane grid is taller than its on-screen rect the render bottom-anchors it
//! via `downscale_rows_v2`, which drops `start` rows off the top. The
//! post-draw cursor write must subtract the same offset — measured on a Devin
//! Windows box: pane 120x50, prompt on pane row 37, client at 120x38 put the
//! caret on the row below the prompt (`\x1b[37;23H` while the prompt cell
//! drew at row 36).

use crate::client::{
    active_pane_clip_offset, clipped_pane_cursor_pos, downscale_rows_v2,
};
use crate::layout::{CellRunJson, LayoutJson, RowRunsJson};
use ratatui::layout::Rect;

fn row(text: &str) -> RowRunsJson {
    RowRunsJson {
        runs: vec![CellRunJson {
            text: text.to_string(),
            fg: "default".to_string(),
            bg: "default".to_string(),
            flags: 0,
            width: 1,
            link: None,
            ul: 0,
            ulc: None,
        }],
    }
}

fn blank_row() -> RowRunsJson {
    row(" ")
}

fn pane(id: usize, rows: u16, cols: u16, cr: u16, cc: u16, content: Vec<RowRunsJson>) -> LayoutJson {
    LayoutJson::Leaf {
        id,
        rows,
        cols,
        cursor_row: cr,
        cursor_col: cc,
        alternate_screen: false,
        wants_mouse: false,
        hide_cursor: false,
        cursor_shape: 0,
        active: true,
        copy_mode: false,
        scroll_offset: 0,
        view_offset: 0,
        sel_start_row: None,
        sel_start_col: None,
        sel_end_row: None,
        sel_end_col: None,
        sel_mode: None,
        copy_cursor_row: None,
        copy_cursor_col: None,
        content: Vec::new(),
        rows_v2: content,
        title: None,
    }
}

#[test]
fn bottom_anchor_matches_render_rows() {
    // 50-row pane, prompt on row 37, blank rows after: the blank-trim keeps
    // one row past the text, then the bottom `dst_h` of what remains.
    let mut src = Vec::new();
    for i in 0..38 {
        src.push(row(&format!("line{i}")));
    }
    for _ in 38..50 {
        src.push(blank_row());
    }
    // kept = 39 (38 used + 1 blank for the cursor line); start = 39 - 37 = 2.
    let inner = Rect::new(0, 0, 120, 37);
    let layout = pane(0, 50, 120, 37, 22, src.clone());
    assert_eq!(active_pane_clip_offset(&layout, inner), 2);

    // The caret must land on the row the prompt cell was drawn at:
    // pane row 37 -> inner.y + 37 - 2 = 35, not inner.y + min(37, 36) = 36.
    let out = downscale_rows_v2(&src, 50, 120, inner.height, inner.width);
    assert_eq!(out.len(), 37);
    assert_eq!(out[35].runs[0].text, "line37");
    assert_eq!(clipped_pane_cursor_pos(inner, 2, 22, 37), Some((22, 35)));
}

#[test]
fn one_row_taller_lands_the_caret_on_the_prompt_row() {
    // The reported shape: window exactly one row taller than the client —
    // prompt on the last pane row, caret must share its row, not sit below.
    let mut src = Vec::new();
    for i in 0..38 {
        src.push(row(&format!("line{i}")));
    }
    let inner = Rect::new(0, 0, 120, 37);
    let layout = pane(0, 38, 120, 37, 22, src);
    assert_eq!(active_pane_clip_offset(&layout, inner), 1);
    assert_eq!(clipped_pane_cursor_pos(inner, 1, 22, 37), Some((22, 36)));
}

#[test]
fn fitting_pane_is_unchanged() {
    // No downscale: clip is 0 and the mapping is the identity inside inner.
    let src: Vec<RowRunsJson> = (0..24).map(|i| row(&format!("line{i}"))).collect();
    let inner = Rect::new(0, 0, 80, 24);
    let layout = pane(0, 24, 80, 3, 10, src);
    assert_eq!(active_pane_clip_offset(&layout, inner), 0);
    assert_eq!(clipped_pane_cursor_pos(inner, 0, 10, 3), Some((10, 3)));
    assert_eq!(clipped_pane_cursor_pos(inner, 0, 10, 23), Some((10, 23)));
}

#[test]
fn cursor_scrolled_out_of_view_hides() {
    // Pane cursor in the clipped-away band: parking it on the first visible
    // row would draw a caret nowhere near its text.
    let mut src = Vec::new();
    for i in 0..50 {
        src.push(row(&format!("line{i}")));
    }
    let inner = Rect::new(0, 0, 120, 37);
    let layout = pane(0, 50, 120, 0, 22, src);
    // all rows non-blank: start = 50 - 37 = 13
    assert_eq!(active_pane_clip_offset(&layout, inner), 13);
    assert_eq!(clipped_pane_cursor_pos(inner, 13, 22, 0), None, "cursor above the visible band");
    assert_eq!(clipped_pane_cursor_pos(inner, 13, 22, 12), None, "last clipped row still hidden");
    assert_eq!(clipped_pane_cursor_pos(inner, 13, 22, 13), Some((22, 0)));
    assert_eq!(clipped_pane_cursor_pos(inner, 13, 22, 49), Some((22, 36)));
}

#[test]
fn only_the_active_leaves_clip_counts() {
    // Leaf 0 is oversized but INACTIVE, leaf 1 is active and fits; the
    // offset for the cursor must come from the active pane only.
    let mut big = pane(0, 50, 120, 49, 5, (0..50).map(|i| row(&format!("b{i}"))).collect());
    if let LayoutJson::Leaf { ref mut active, .. } = big {
        *active = false;
    }
    let small = pane(1, 24, 80, 3, 10, (0..24).map(|i| row(&format!("s{i}"))).collect());
    let layout = LayoutJson::Split {
        kind: "Vertical".to_string(),
        sizes: vec![50, 50],
        children: vec![big, small],
    };
    let inner = Rect::new(0, 0, 80, 24);
    assert_eq!(active_pane_clip_offset(&layout, inner), 0);
}

#[test]
fn degenerate_inner_never_panics() {
    let src: Vec<RowRunsJson> = (0..50).map(|i| row(&format!("line{i}"))).collect();
    let layout = pane(0, 50, 120, 49, 5, src);
    assert_eq!(active_pane_clip_offset(&layout, Rect::new(0, 0, 120, 0)), 0);
    assert_eq!(clipped_pane_cursor_pos(Rect::new(0, 0, 120, 0), 0, 0, 0), None);
}
