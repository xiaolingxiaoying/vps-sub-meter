//! QR code helpers: terminal ANSI half-blocks and standalone SVG documents.

use qrcode::{Color, QrCode, render::svg};

const QUIET_ZONE: usize = 4;
const BLACK_ON_WHITE: &str = "\x1b[0;30;47m";
const RESET: &str = "\x1b[0m";

pub fn render_ansi(data: &str) -> Result<String, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|error| error.to_string())?;
    let modules = code.to_colors();
    let symbol_width = code.width();
    let width = symbol_width + 2 * QUIET_ZONE;
    let dark = |row: usize, col: usize| {
        (QUIET_ZONE..QUIET_ZONE + symbol_width).contains(&row)
            && (QUIET_ZONE..QUIET_ZONE + symbol_width).contains(&col)
            && modules[(row - QUIET_ZONE) * symbol_width + col - QUIET_ZONE] == Color::Dark
    };
    let mut out = String::new();
    let mut row = 0;
    while row < width {
        out.push_str(BLACK_ON_WHITE);
        for col in 0..width {
            let top = dark(row, col);
            let bottom = dark(row + 1, col);
            let ch = match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            out.push(ch);
        }
        out.push_str(RESET);
        out.push('\n');
        row += 2;
    }
    Ok(out)
}

pub fn render_svg(data: &str) -> Result<String, String> {
    let code = QrCode::new(data.as_bytes()).map_err(|error| error.to_string())?;
    Ok(code
        .render::<svg::Color>()
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ansi_and_svg() {
        let url = "https://example.com/sub/token/clash.yaml";
        let ansi = render_ansi(url).unwrap();
        assert!(ansi.contains('█'));
        let svg = render_svg(url).unwrap();
        assert!(svg.contains("<svg") && svg.contains("</svg>"));
    }
}
