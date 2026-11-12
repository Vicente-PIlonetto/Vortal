//! Embedded Vortal desktop UI assets and native window.

#[cfg(target_os = "windows")]
mod windows_backend;
#[cfg(target_os = "windows")]
mod windows_clipboard;
#[cfg(target_os = "windows")]
mod windows_native;
#[cfg(target_os = "windows")]
mod windows_tray;

#[cfg(target_os = "windows")]
pub use windows_native::run_desktop;

#[cfg(target_os = "windows")]
const INDEX_HTML: &[u8] = include_bytes!("../ui/index.html");
#[cfg(target_os = "windows")]
const TOKENS_CSS: &[u8] = include_bytes!("../ui/tokens.css");
#[cfg(target_os = "windows")]
const APP_CSS: &[u8] = include_bytes!("../ui/app.css");
#[cfg(target_os = "windows")]
const APP_JS: &[u8] = include_bytes!("../ui/app.js");
#[cfg(target_os = "windows")]
const EN_JSON: &[u8] = include_bytes!("../ui/locales/en.json");
#[cfg(target_os = "windows")]
const PT_BR_JSON: &[u8] = include_bytes!("../ui/locales/pt-BR.json");
#[cfg(target_os = "windows")]
const UI_ICON: &[u8] = include_bytes!("../ui/assets/vortal.png");

#[cfg(target_os = "linux")]
mod linux_native;

#[cfg(target_os = "linux")]
pub use linux_native::run_desktop;

/// Directory containing the standalone UI prototype.
pub const UI_ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/ui");

#[cfg(target_os = "windows")]
const ICON_PNG: &[u8] = include_bytes!("../assets/vortal.png");

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn run_desktop() -> anyhow::Result<()> {
    anyhow::bail!("a interface desktop nativa ainda não está disponível neste sistema")
}

#[cfg(target_os = "windows")]
fn png_to_rgba(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info()?;
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("tamanho do ícone excede o limite"))?;
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer)?;
    anyhow::ensure!(
        info.width == 128 && info.height == 128,
        "ícone Vortal inválido"
    );
    let bytes = &buffer[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(128 * 128 * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(bytes),
        png::ColorType::Rgb => {
            for pixel in bytes.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        other => anyhow::bail!("formato do ícone não suportado: {other:?}"),
    }
    Ok(rgba)
}
