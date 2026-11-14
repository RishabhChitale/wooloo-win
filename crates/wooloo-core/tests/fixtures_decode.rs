use wooloo_core::surface::{drain_frames, frame, Decoder, Frame};

fn fx(i: usize) -> Vec<u8> {
    let p = format!("{}/tests/fixtures/frame_{:03}.bin", env!("CARGO_MANIFEST_DIR"), i);
    std::fs::read(p).unwrap_or_else(|e| panic!("fixture {i}: {e}"))
}

/// Fixture frames may be stored with or without the 4-byte length prefix.
fn payload(i: usize) -> Vec<u8> {
    let b = fx(i);
    if b.len() > 4 && u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize == b.len() - 4 {
        b[4..].to_vec()
    } else {
        b
    }
}

#[test]
fn decodes_real_stream() {
    let mut d = Decoder::new();
    let mut kinds = Vec::new();
    for i in 0..14 {
        kinds.push(d.apply(&payload(i)).unwrap_or_else(|e| panic!("frame {i}: {e}")));
    }
    assert!(matches!(&kinds[0], Frame::Control { kind, .. } if kind == "endpoint.welcome.v1"));
    assert!(matches!(&kinds[2], Frame::Control { kind, .. } if kind == "shell.snapshot.v1"));
    assert!(matches!(kinds[3], Frame::Ignored(8)));
    assert!(matches!(kinds[6], Frame::Full));
    for i in [7, 8, 9, 10, 11, 13] {
        assert!(matches!(kinds[i], Frame::Patch { .. }), "frame {i}: {:?}", kinds[i]);
    }
    assert!(matches!(kinds[12], Frame::Patch { .. }), "scroll frame: {:?}", kinds[12]);
    let sf = d.surface.unwrap();
    assert_eq!(sf.cells.len(), sf.width * sf.height);
    assert!(!sf.panes.is_empty());
}

#[test]
fn full_surface_dimensions() {
    let mut d = Decoder::new();
    d.apply(&payload(6)).unwrap();
    let sf = d.surface.unwrap();
    assert_eq!(sf.cells.len(), sf.width * sf.height);
}

#[test]
fn patch_before_surface_is_error() {
    let mut d = Decoder::new();
    assert!(d.apply(&payload(7)).is_err());
}

#[test]
fn frame_roundtrip() {
    let mut buf = frame(b"abc");
    buf.extend(frame(b"de"));
    buf.extend_from_slice(&[9, 0]);
    let out = drain_frames(&mut buf).unwrap();
    assert_eq!(out, vec![b"abc".to_vec(), b"de".to_vec()]);
    assert_eq!(buf, vec![9, 0]);
}

/// Frame 7 carries the `printf` line with "中文". A wide cell is followed by exactly one cell with an
/// EMPTY symbol (not skip), so columns stay aligned with the grid.
#[test]
fn wide_cell_is_followed_by_empty_spacer() {
    use wooloo_core::surface::out_cell;
    let mut d = Decoder::new();
    for i in 0..=7 {
        d.apply(&payload(i)).unwrap_or_else(|e| panic!("frame {i}: {e}"));
    }
    let sf = d.surface.unwrap();
    let i = sf.cells.iter().position(|c| c.symbol == "中").expect("wide char in fixture");
    assert_eq!(sf.cells[i + 1].symbol, "", "spacer after wide cell");
    assert!(!sf.cells[i + 1].skip);
    assert_eq!(sf.cells[i + 2].symbol, "文");
    assert_eq!(sf.cells[i + 3].symbol, "");
    let (w, sp) = (out_cell(&sf.cells[i]), out_cell(&sf.cells[i + 1]));
    assert!(w.wide && w.ch == "中");
    assert!(!sp.wide && sp.ch.is_empty());
}
