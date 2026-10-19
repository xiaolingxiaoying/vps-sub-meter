//! QR rendering helpers built on the `qrcode` crate: terminal ANSI half-blocks
//! for `sbctl qr` and standalone SVG documents for the subscription QR links.

use qrcode::{Color, QrCode, render::svg};

// Model 2 QR symbols require four light modules on every side. Keep the
// background local to each row: a newline under an active background color
// can paint the rest of the terminal and obscure the symbol's boundary.
const QUIET_ZONE: usize = 4;
const BLACK_ON_WHITE: &str = "\x1b[0;30;47m";
const RESET: &str = "\x1b[0m";

/// Render a terminal ANSI QR code for `data` using black-on-white UTF-8
/// half-blocks, which keeps the square roughly half as tall as a naive two-row
/// rendering. Returns a string with embedded ANSI escape codes suitable for
/// printing to an interactive terminal.
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
        let mut line = String::new();
        for col in 0..width {
            let top = dark(row, col);
            let bottom = dark(row + 1, col);
            let ch = match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            line.push(ch);
        }
        out.push_str(&line);
        out.push_str(RESET);
        out.push('\n');
        row += 2;
    }
    Ok(out)
}

/// Render a self-contained SVG document (black modules on white, quiet zone
/// included) so a subscription QR link can be scanned from any browser screen.
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
    fn renders_a_scan_friendly_qr_for_a_url() {
        let rendered = render_ansi("https://example.com/sub/abc").expect("qr should render");
        assert!(rendered.contains(BLACK_ON_WHITE), "missing ANSI start");
        assert!(rendered.contains("\x1b[0m"), "missing ANSI reset");
        assert!(rendered.contains('█'), "missing half-block modules");
    }

    #[test]
    fn terminal_qr_preserves_every_module_and_a_four_module_quiet_zone() {
        // Include both short links and dense single-node links. Every QR
        // version has an odd module count, so the last half-row must be light.
        for payload in [
            "https://example.com/sub/abc".to_owned(),
            "vmess://".to_owned() + &"a".repeat(600),
        ] {
            let code = QrCode::new(payload.as_bytes()).unwrap();
            let rendered = render_ansi(&payload).unwrap();
            let width = code.width() + 8;
            let mut pixels = Vec::new();
            for row in rendered.lines() {
                let cells: Vec<_> = row
                    .strip_prefix(BLACK_ON_WHITE)
                    .unwrap()
                    .strip_suffix(RESET)
                    .unwrap()
                    .chars()
                    .collect();
                assert_eq!(cells.len(), width);
                pixels.push(
                    cells
                        .iter()
                        .map(|ch| matches!(ch, '█' | '▀'))
                        .collect::<Vec<_>>(),
                );
                pixels.push(
                    cells
                        .iter()
                        .map(|ch| matches!(ch, '█' | '▄'))
                        .collect::<Vec<_>>(),
                );
            }
            assert_eq!(pixels.len(), width + 1);
            for (y, row) in pixels.iter().enumerate() {
                for (x, &pixel) in row.iter().enumerate() {
                    let expected = if (4..width - 4).contains(&x) && (4..width - 4).contains(&y) {
                        code[(x - 4, y - 4)] == Color::Dark
                    } else {
                        false
                    };
                    assert_eq!(pixel, expected, "module ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn renders_a_parseable_svg_for_the_same_payload_as_the_terminal_code() {
        let url = "https://example.com/sub/abc/index";
        let svg = render_svg(url).expect("svg should render");
        assert!(svg.contains("<svg"), "missing svg root element");
        assert!(svg.contains("</svg>"), "missing svg close tag");
        assert!(svg.contains("width") && svg.contains("height"));
        assert_eq!(svg, render_svg(url).expect("svg is deterministic"));
        // The same payload renders in both representations, and a different
        // payload produces a different document.
        assert!(render_ansi(url).is_ok());
        assert_ne!(
            svg,
            render_svg("https://example.com/sub/other").expect("svg renders")
        );
    }
}
