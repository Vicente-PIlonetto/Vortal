use windows_sys::Win32::System::{
    DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW},
    Memory::{GlobalLock, GlobalSize, GlobalUnlock},
};

const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024 - 16 * 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Option<Self> {
        // SAFETY: A null owner is valid when reading the system clipboard.
        (unsafe { OpenClipboard(std::ptr::null_mut()) } != 0).then_some(Self)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: ClipboardGuard owns the matching OpenClipboard call.
        unsafe { CloseClipboard() };
    }
}

pub(super) fn read_image() -> Option<Vec<u8>> {
    let _guard = ClipboardGuard::open()?;
    // SAFETY: RegisterClipboardFormatW reads a terminated static UTF-16 string.
    let png_format = unsafe { RegisterClipboardFormatW([80, 78, 71, 0].as_ptr()) };
    if png_format != 0 {
        if let Some(bytes) = read_format(png_format) {
            if bytes.starts_with(PNG_SIGNATURE) {
                return Some(bytes);
            }
        }
    }
    read_format(17)
        .or_else(|| read_format(8))
        .and_then(|dib| encode_dib(&dib))
}

fn read_format(format: u32) -> Option<Vec<u8>> {
    // SAFETY: The clipboard remains open while its global memory is locked and copied.
    unsafe {
        let handle = GetClipboardData(format);
        if handle.is_null() {
            return None;
        }
        let size = GlobalSize(handle);
        if size == 0 || size > 128 * 1024 * 1024 {
            return None;
        }
        let pointer = GlobalLock(handle) as *const u8;
        if pointer.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
        GlobalUnlock(handle);
        Some(bytes)
    }
}

fn encode_dib(dib: &[u8]) -> Option<Vec<u8>> {
    let header_size = read_u32(dib, 0)? as usize;
    if header_size < 40 || header_size > dib.len() {
        return None;
    }
    let width = i32::from_le_bytes(dib.get(4..8)?.try_into().ok()?);
    let height = i32::from_le_bytes(dib.get(8..12)?.try_into().ok()?);
    let planes = u16::from_le_bytes(dib.get(12..14)?.try_into().ok()?);
    let depth = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let compression = read_u32(dib, 16)?;
    if width <= 0
        || height == 0
        || planes != 1
        || !matches!(depth, 24 | 32)
        || !matches!(compression, 0 | 3)
    {
        return None;
    }
    if compression == 3 {
        let masks = dib.get(40..52)?;
        if masks != [0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0] {
            return None;
        }
    }
    let (width, height_abs) = (width as usize, height.unsigned_abs() as usize);
    if width > 8192 || height_abs > 8192 || width.checked_mul(height_abs)? > 16 * 1024 * 1024 {
        return None;
    }
    let bytes_per_pixel = usize::from(depth / 8);
    let stride = width.checked_mul(bytes_per_pixel)?.checked_add(3)? & !3;
    let pixel_offset = if compression == 3 && header_size == 40 {
        header_size + 12
    } else {
        header_size
    };
    let pixel_bytes = stride.checked_mul(height_abs)?;
    let pixels = dib.get(pixel_offset..pixel_offset.checked_add(pixel_bytes)?)?;
    let mut rgba = vec![0u8; width.checked_mul(height_abs)?.checked_mul(4)?];
    let has_alpha = depth == 32 && pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    for row in 0..height_abs {
        let source_row = if height > 0 {
            height_abs - row - 1
        } else {
            row
        };
        for col in 0..width {
            let source = (source_row * stride) + col * bytes_per_pixel;
            let target = (row * width + col) * 4;
            rgba[target] = pixels[source + 2];
            rgba[target + 1] = pixels[source + 1];
            rgba[target + 2] = pixels[source];
            rgba[target + 3] = if has_alpha { pixels[source + 3] } else { 255 };
        }
    }
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width as u32, height_abs as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    {
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    (png.len() <= MAX_IMAGE_BYTES).then_some(png)
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
