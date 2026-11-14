//! Binary generation-1 client endpoint: frame reader/writer, surface decoder, input encoder.
//! Mirrors `HerdrSurface.swift` from the Swift wooloo client.
use crate::{Error, Result};
use serde::Serialize;

pub const MAX_FRAME: usize = 32 * 1024 * 1024;
pub const SCROLL_KIND: &str = "endpoint.surface-scroll.v1";

fn bad<T>(what: &str) -> Result<T> {
    Err(Error::protocol(what.to_string()))
}

// ------------------------------------------------------------------ reader

pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    pub fn byte(&mut self) -> Result<u8> {
        let v = *self.b.get(self.pos).ok_or_else(|| Error::protocol("unexpected end"))?;
        self.pos += 1;
        Ok(v)
    }
    pub fn number(&mut self) -> Result<u64> {
        let tag = self.byte()?;
        let n = match tag {
            0..=250 => return Ok(tag as u64),
            251 => 2,
            252 => 4,
            253 => 8,
            _ => return bad("invalid number tag"),
        };
        if self.pos + n > self.b.len() {
            return bad("unexpected end");
        }
        let mut v = 0u64;
        for i in 0..n {
            v |= (self.b[self.pos + i] as u64) << (i * 8);
        }
        self.pos += n;
        Ok(v)
    }
    fn usize(&mut self) -> Result<usize> {
        let v = self.number()?;
        usize::try_from(v).or_else(|_| bad("number too large"))
    }
    pub fn string(&mut self) -> Result<String> {
        let len = self.usize()?;
        if len > 8_000_000 || self.pos + len > self.b.len() {
            return bad("bad string length");
        }
        let s = std::str::from_utf8(&self.b[self.pos..self.pos + len]).or_else(|_| bad("invalid utf-8"))?;
        self.pos += len;
        Ok(s.to_string())
    }
    pub fn count(&mut self) -> Result<usize> {
        let v = self.number()?;
        if v > 1_000_000 {
            return bad("count too large");
        }
        Ok(v as usize)
    }
    pub fn data(&mut self) -> Result<&'a [u8]> {
        let len = self.usize()?;
        if len > MAX_FRAME || len > self.b.len() - self.pos {
            return bad("bad data length");
        }
        let d = &self.b[self.pos..self.pos + len];
        self.pos += len;
        Ok(d)
    }
    pub fn signed(&mut self) -> Result<i64> {
        let e = self.number()?;
        if e > i64::MAX as u64 {
            return bad("signed overflow");
        }
        Ok(if e & 1 == 0 { (e / 2) as i64 } else { -((e / 2) as i64) - 1 })
    }
    pub fn flag(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => bad("invalid option byte"),
        }
    }
    pub fn rect(&mut self) -> Result<Rect> {
        Ok(Rect { x: self.usize()?, y: self.usize()?, w: self.usize()?, h: self.usize()? })
    }
    pub fn cell(&mut self) -> Result<Cell> {
        let symbol = self.string()?;
        let fg = self.number()? as u32;
        let bg = self.number()? as u32;
        let modifier = self.number()? as u16;
        let skip = self.byte()? != 0;
        let hyperlink = if self.flag()? { Some(self.number()?) } else { None };
        Ok(Cell { symbol, fg, bg, modifier, skip, hyperlink })
    }
    pub fn cursor(&mut self) -> Result<Cursor> {
        Ok(Cursor { x: self.usize()?, y: self.usize()?, visible: self.byte()? != 0, shape: self.byte()? })
    }
    fn opt_cursor(&mut self) -> Result<Option<Cursor>> {
        if self.flag()? { Ok(Some(self.cursor()?)) } else { Ok(None) }
    }
    pub fn pane(&mut self) -> Result<PaneInfo> {
        let id = self.string()?;
        self.number()?; // content revision
        let rect = self.rect()?;
        let inner = self.rect()?;
        if self.flag()? {
            self.rect()?;
        }
        if self.flag()? {
            for _ in 0..3 {
                self.number()?;
            }
        }
        let focused = self.byte()? != 0;
        let mouse_reporting = self.byte()? != 0;
        self.byte()?; // sgr pixel
        self.byte()?; // alt screen
        self.number()?;
        self.number()?;
        Ok(PaneInfo { id, rect, inner, focused, mouse_reporting })
    }
    fn split(&mut self) -> Result<()> {
        if self.number()? > 1 {
            return bad("bad split direction");
        }
        self.number()?;
        self.rect()?;
        self.rect()?;
        for _ in 0..self.count()? {
            if self.byte()? > 1 {
                return bad("bad split path");
            }
        }
        Ok(())
    }
    fn popup(&mut self) -> Result<()> {
        let id = self.string()?;
        self.string()?; // title
        for _ in 0..2 {
            if self.flag()? {
                let kind = self.number()?;
                let value = self.number()?;
                let ok = (kind == 0 && value <= u16::MAX as u64) || (kind == 1 && value > 0 && value <= 100);
                if !ok {
                    return bad("bad popup size");
                }
            }
        }
        let len = self.count()?;
        if len > 200_000 {
            return bad("popup too large");
        }
        for _ in 0..len {
            self.cell()?;
        }
        let cols = self.number()?;
        let rows = self.number()?;
        if id.is_empty() || cols == 0 || rows == 0 || cols > 200_000 || rows > 200_000 || cols * rows != len as u64 {
            return bad("bad popup dimensions");
        }
        self.opt_cursor()?;
        for _ in 0..self.count()? {
            self.string()?;
        }
        self.data()?;
        if self.byte()? > 1 || self.byte()? > 1 {
            return bad("bad popup flags");
        }
        self.number()?;
        self.number()?;
        Ok(())
    }
    /// Reads a graphic key, returning the declared data length.
    fn graphic_key(&mut self) -> Result<usize> {
        match self.number()? {
            0 => {
                if self.number()? > 1 {
                    return bad("bad graphic key");
                }
                self.string()?;
                self.number()?;
            }
            1 => {
                self.string()?;
                self.string()?;
            }
            _ => return bad("bad graphic key"),
        }
        let w = self.number()?;
        let h = self.number()?;
        self.number()?; // format
        let len = self.number()?;
        if w == 0 || h == 0 || w > 16_384 || h > 16_384 || len > MAX_FRAME as u64 {
            return bad("bad graphic key");
        }
        self.number()?; // fingerprint
        Ok(len as usize)
    }
    fn graphic_placement(&mut self) -> Result<()> {
        self.graphic_key()?;
        for _ in 0..11 {
            self.number()?;
        }
        self.signed()?;
        self.number()?;
        Ok(())
    }
}

// ------------------------------------------------------------------ model

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub symbol: String,
    pub fg: u32,
    pub bg: u32,
    pub modifier: u16,
    pub skip: bool,
    pub hyperlink: Option<u64>,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { symbol: " ".into(), fg: 0, bg: 0, modifier: 0, skip: false, hyperlink: None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cursor {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
    pub shape: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub id: String,
    pub rect: Rect,
    pub inner: Rect,
    pub focused: bool,
    pub mouse_reporting: bool,
}

#[derive(Debug, Clone)]
pub struct Surface {
    pub boot_id: String,
    pub projection_revision: u64,
    pub revision: u64,
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<Cursor>,
    pub hyperlinks: Vec<String>,
    pub panes: Vec<PaneInfo>,
}

impl Surface {
    pub fn row(&self, y: usize) -> &[Cell] {
        &self.cells[y * self.width..(y + 1) * self.width]
    }
}

// ------------------------------------------------------------------ scroll

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scroll {
    pub rect: Rect,
    pub shift: i32,
}

impl Scroll {
    pub fn fits(&self, width: usize, height: usize) -> bool {
        let r = self.rect;
        r.w > 0
            && r.h >= 2
            && self.shift != 0
            && (self.shift.unsigned_abs() as usize) < r.h
            && r.x + r.w <= width
            && r.y + r.h <= height
    }

    pub fn disjoint(s: &[Scroll]) -> bool {
        for i in 0..s.len() {
            for o in &s[i + 1..] {
                let (a, b) = (s[i].rect, o.rect);
                if a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h {
                    return false;
                }
            }
        }
        true
    }

    /// Swaps rows exactly like Herdr's `for_each_swap`.
    pub fn apply(&self, sf: &mut Surface) {
        let d = self.shift.unsigned_abs() as usize;
        let h = self.rect.h;
        let pairs: Vec<(usize, usize)> = if self.shift > 0 {
            (0..h - d).map(|i| (i, i + d)).collect()
        } else {
            (d..h).rev().map(|i| (i, i - d)).collect()
        };
        for (a, b) in pairs {
            let (ra, rb) = (self.rect.y + a, self.rect.y + b);
            for x in self.rect.x..self.rect.x + self.rect.w {
                sf.cells.swap(ra * sf.width + x, rb * sf.width + x);
            }
        }
    }
}

// ------------------------------------------------------------------ decoder

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Complete surface replaced the grid.
    Full,
    /// Patch / scrolled patch applied; these rows changed.
    Patch { dirty_rows: Vec<usize> },
    /// Control frame other than surface-scroll.
    Control { kind: String, data: String },
    /// Clipboard (tag 5), base64 payload.
    Clipboard(String),
    /// Known-ignorable or unknown tag.
    Ignored(u64),
}

#[derive(Default)]
pub struct Decoder {
    pub surface: Option<Surface>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, frame: &[u8]) -> Result<Frame> {
        let mut r = Reader::new(frame);
        let tag = r.number()?;
        match tag {
            13 => {
                self.surface = Some(read_surface(&mut r)?);
                Ok(Frame::Full)
            }
            19 => {
                let sf = self.surface.as_mut().ok_or_else(|| Error::protocol("patch before surface"))?;
                let mut work = sf.clone();
                let dirty = apply_patch(&mut r, &mut work, &[])?;
                *sf = work;
                Ok(Frame::Patch { dirty_rows: dirty })
            }
            20 => {
                let kind = r.string()?;
                let data = r.string()?;
                if kind == SCROLL_KIND {
                    self.apply_scroll(&data)
                } else {
                    Ok(Frame::Control { kind, data })
                }
            }
            5 => Ok(Frame::Clipboard(r.string()?)),
            t => Ok(Frame::Ignored(t)),
        }
    }

    fn apply_scroll(&mut self, data: &str) -> Result<Frame> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(data.trim_end_matches('='))
            .or_else(|_| bad("bad scroll base64"))?;
        let count = *bytes.first().ok_or_else(|| Error::protocol("empty scroll"))? as usize;
        if !(1..=64).contains(&count) {
            return bad("bad scroll count");
        }
        let header = 1 + count * 10;
        if bytes.len() < header + 4 {
            return bad("short scroll");
        }
        let u16at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let scrolls: Vec<Scroll> = (0..count)
            .map(|k| {
                let a = 1 + k * 10;
                Scroll {
                    rect: Rect { x: u16at(a) as usize, y: u16at(a + 2) as usize, w: u16at(a + 4) as usize, h: u16at(a + 6) as usize },
                    shift: u16at(a + 8) as i16 as i32,
                }
            })
            .collect();
        let len = u32::from_le_bytes([bytes[header], bytes[header + 1], bytes[header + 2], bytes[header + 3]]) as usize;
        if header + 4 + len != bytes.len() {
            return bad("scroll length mismatch");
        }
        let mut r = Reader::new(&bytes[header + 4..]);
        if r.number()? != 19 {
            return bad("scroll payload is not a patch");
        }
        let sf = self.surface.as_mut().ok_or_else(|| Error::protocol("scroll before surface"))?;
        let mut work = sf.clone();
        let mut dirty = apply_patch(&mut r, &mut work, &scrolls)?;
        for s in &scrolls {
            dirty.extend(s.rect.y..s.rect.y + s.rect.h);
        }
        dirty.sort_unstable();
        dirty.dedup();
        *sf = work;
        Ok(Frame::Patch { dirty_rows: dirty })
    }
}

fn read_surface(r: &mut Reader) -> Result<Surface> {
    let boot_id = r.string()?;
    let projection_revision = r.number()?;
    let revision = r.number()?;
    let n = r.count()?;
    if n > 200_000 {
        return bad("too many cells");
    }
    let mut cells = Vec::with_capacity(n);
    for _ in 0..n {
        cells.push(r.cell()?);
    }
    let width = r.usize()?;
    let height = r.usize()?;
    if width == 0 || height == 0 || width.checked_mul(height) != Some(n) {
        return bad("surface dimensions do not match cells");
    }
    let cursor = r.opt_cursor()?;
    let mut hyperlinks = Vec::new();
    for _ in 0..r.count()? {
        hyperlinks.push(r.string()?);
    }
    for _ in 0..r.count()? {
        r.byte()?; // legacy graphics bytes
    }
    let mut panes = Vec::new();
    for _ in 0..r.count()? {
        panes.push(r.pane()?);
    }
    for _ in 0..r.count()? {
        r.split()?;
    }
    if r.flag()? {
        r.popup()?;
    }
    for _ in 0..r.count()? {
        let len = r.graphic_key()?;
        if r.data()?.len() != len {
            return bad("graphic length mismatch");
        }
    }
    for _ in 0..r.count()? {
        r.graphic_placement()?;
    }
    for _ in 0..r.count()? {
        r.graphic_key()?;
    }
    Ok(Surface { boot_id, projection_revision, revision, width, height, cells, cursor, hyperlinks, panes })
}

fn apply_patch(r: &mut Reader, sf: &mut Surface, scrolls: &[Scroll]) -> Result<Vec<usize>> {
    let boot_id = r.string()?;
    let proj = r.number()?;
    let base = r.number()?;
    let revision = r.number()?;
    if boot_id != sf.boot_id || proj != sf.projection_revision || base != sf.revision {
        return bad("patch does not match surface");
    }
    if !scrolls.iter().all(|s| s.fits(sf.width, sf.height)) || !Scroll::disjoint(scrolls) {
        return bad("invalid scroll");
    }
    for s in scrolls {
        s.apply(sf);
    }
    let mut dirty = Vec::new();
    for _ in 0..r.count()? {
        let x = r.usize()?;
        let y = r.usize()?;
        let len = r.count()?;
        if y >= sf.height || x + len > sf.width {
            return bad("patch run out of bounds");
        }
        for o in 0..len {
            sf.cells[y * sf.width + x + o] = r.cell()?;
        }
        dirty.push(y);
    }
    for _ in 0..r.count()? {
        let p = r.pane()?;
        let slot = sf.panes.iter_mut().find(|q| q.id == p.id).ok_or_else(|| Error::protocol("patch for unknown pane"))?;
        *slot = p;
    }
    sf.cursor = r.opt_cursor()?;
    sf.revision = revision;
    dirty.sort_unstable();
    dirty.dedup();
    Ok(dirty)
}

// ------------------------------------------------------------------ frame I/O

/// Splits complete frames out of `buf`, returning each payload. Leaves partial data in place.
pub fn drain_frames(buf: &mut Vec<u8>) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while buf.len() - pos >= 4 {
        let len = u32::from_le_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]]) as usize;
        if len == 0 || len > MAX_FRAME {
            return bad("bad frame length");
        }
        if buf.len() - pos - 4 < len {
            break;
        }
        out.push(buf[pos + 4..pos + 4 + len].to_vec());
        pos += 4 + len;
    }
    buf.drain(..pos);
    Ok(out)
}

pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = (payload.len() as u32).to_le_bytes().to_vec();
    v.extend_from_slice(payload);
    v
}

// ------------------------------------------------------------------ writer (client -> server)

pub fn put_number(out: &mut Vec<u8>, v: u64) {
    if v < 251 {
        out.push(v as u8);
    } else if v <= u16::MAX as u64 {
        out.push(251);
        out.extend_from_slice(&(v as u16).to_le_bytes());
    } else if v <= u32::MAX as u64 {
        out.push(252);
        out.extend_from_slice(&(v as u32).to_le_bytes());
    } else {
        out.push(253);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

pub fn put_string(out: &mut Vec<u8>, s: &str) {
    put_number(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

pub fn control(kind: &str, data: &str) -> Vec<u8> {
    let mut o = Vec::new();
    put_number(&mut o, 20);
    put_string(&mut o, kind);
    put_string(&mut o, data);
    o
}

pub fn hello(cell_w: u32, cell_h: u32, cols: u32, rows: u32) -> Vec<u8> {
    let codecs = |c: &str| serde_json::json!([c]);
    let j = serde_json::json!({
        "generation": 1, "cell_width_px": cell_w, "cell_height_px": cell_h,
        "surface_size": {"cols": cols, "rows": rows},
        "pixel_mouse": false, "direct_graphics": false, "endpoint_keybindings": false,
        "mouse_capture": false, "surface_active": true, "surface_reuse": false,
        "surface_delta": false, "surface_scroll": true,
        "snapshot_codecs": codecs("shell.snapshot.v1"), "surface_codecs": codecs("shell.surface.v1"),
        "input_codecs": codecs("shell.input.semantic.v1"), "blob_codecs": codecs("shell.blob.v1"),
    });
    control("endpoint.hello.v1", &j.to_string())
}

pub fn resize(cell_w: u32, cell_h: u32, cols: u32, rows: u32) -> Vec<u8> {
    let mut o = Vec::new();
    put_number(&mut o, 12);
    for v in [cell_w, cell_h, cols, rows] {
        put_number(&mut o, v as u64);
    }
    o.push(0);
    o
}

pub fn focus(active: bool) -> Vec<u8> {
    let mut o = Vec::new();
    put_number(&mut o, 18);
    o.push(active as u8);
    o
}

pub fn ping(token: &str) -> Vec<u8> {
    control("endpoint.health.ping.v1", token)
}

/// Tag 15: a JSON API request routed through the endpoint (`tab.focus`, `pane.focus`, ...).
pub fn api_request(boot_id: &str, id: &str, method: &str, params: serde_json::Value) -> Vec<u8> {
    let mut o = Vec::new();
    put_number(&mut o, 15);
    put_string(&mut o, boot_id);
    put_string(&mut o, &serde_json::json!({"id": id, "method": method, "params": params}).to_string());
    o
}

fn input_head(pane_id: &str) -> Vec<u8> {
    let mut o = Vec::new();
    put_number(&mut o, 13);
    put_string(&mut o, pane_id);
    put_number(&mut o, 1);
    o
}

pub fn input_text(pane_id: &str, text: &str) -> Vec<u8> {
    let mut o = input_head(pane_id);
    put_number(&mut o, 1);
    put_string(&mut o, text);
    o
}

pub fn input_paste(pane_id: &str, text: &str) -> Vec<u8> {
    let mut o = input_head(pane_id);
    put_number(&mut o, 3);
    put_string(&mut o, text);
    o
}

/// Mouse wheel over a cell: kind 4 = up, 5 = down.
pub fn input_wheel(pane_id: &str, up: bool, col: u32, row: u32, lines: u32) -> Vec<u8> {
    let mut o = input_head(pane_id);
    put_number(&mut o, 2);
    put_number(&mut o, if up { 4 } else { 5 });
    put_number(&mut o, 0);
    put_number(&mut o, col as u64);
    put_number(&mut o, row as u64);
    o.push(0);
    o.push(0);
    put_number(&mut o, lines as u64);
    o
}

/// Named key codes of the semantic input codec.
pub fn key_code(name: &str, shift: bool) -> Option<u64> {
    Some(match name {
        "Backspace" => 0,
        "Enter" => 1,
        "ArrowLeft" | "Left" => 2,
        "ArrowRight" | "Right" => 3,
        "ArrowUp" | "Up" => 4,
        "ArrowDown" | "Down" => 5,
        "Home" => 6,
        "End" => 7,
        "PageUp" => 8,
        "PageDown" => 9,
        "Tab" => if shift { 11 } else { 10 },
        "Delete" => 12,
        "Escape" | "Esc" => 14,
        _ => return None,
    })
}

/// A key event. `code` is a DOM `key` value ("a", "Enter", "ArrowUp"...). Returns None for unsupported keys.
pub fn input_key(pane_id: &str, code: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
    let mut o = input_head(pane_id);
    put_number(&mut o, 0);
    let mut chars = code.chars();
    let (c, rest) = (chars.next()?, chars.next());
    match key_code(code, shift) {
        Some(k) => put_number(&mut o, k),
        None if rest.is_none() => {
            put_number(&mut o, 15);
            let mut b = [0u8; 4];
            o.extend_from_slice(c.to_lowercase().next().unwrap_or(c).encode_utf8(&mut b).as_bytes());
        }
        None => return None,
    }
    o.push((shift as u8) | ((ctrl as u8) << 1) | ((alt as u8) << 2));
    put_number(&mut o, 0); // press
    put_number(&mut o, 1); // repeat count
    o.extend_from_slice(&[0, 0, 1, 0, 0]);
    Some(o)
}

// ------------------------------------------------------------------ colour resolution

const ANSI: [[u8; 3]; 16] = [
    [0, 0, 0], [205, 49, 49], [13, 188, 121], [229, 229, 16], [36, 114, 200], [188, 63, 188], [17, 168, 205], [229, 229, 229],
    [102, 102, 102], [241, 76, 76], [35, 209, 139], [245, 245, 67], [59, 142, 234], [214, 112, 214], [41, 184, 219], [255, 255, 255],
];

/// Resolves a wire colour to RGB; `None` means the terminal default.
pub fn resolve_color(c: u32) -> Option<[u8; 3]> {
    let low = (c & 0xff) as usize;
    match c >> 24 {
        0 => if low == 0 || low > 16 { None } else { Some(ANSI[low - 1]) },
        1 => Some(match low {
            0..=15 => ANSI[low],
            16..=231 => {
                let l = [0u8, 95, 135, 175, 215, 255];
                let i = low - 16;
                [l[i / 36], l[(i / 6) % 6], l[i % 6]]
            }
            _ => {
                let g = (8 + (low - 232) * 10) as u8;
                [g, g, g]
            }
        }),
        2 => Some([(c >> 16) as u8, (c >> 8) as u8, c as u8]),
        _ => None,
    }
}

pub fn is_wide(s: &str) -> bool {
    s.chars().next().map_or(false, |c| {
        let u = c as u32;
        matches!(u, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutCell {
    pub ch: String,
    pub fg: Option<[u8; 3]>,
    pub bg: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub wide: bool,
}

pub fn out_cell(c: &Cell) -> OutCell {
    OutCell {
        ch: if c.skip { String::new() } else { c.symbol.clone() },
        fg: resolve_color(c.fg),
        bg: resolve_color(c.bg),
        bold: c.modifier & 1 != 0,
        italic: c.modifier & 4 != 0,
        underline: c.modifier & 8 != 0,
        inverse: c.modifier & 64 != 0,
        wide: is_wide(&c.symbol),
    }
}
