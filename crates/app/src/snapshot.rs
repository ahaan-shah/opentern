//! Chart snapshots (TradingView's camera button): the charts' area of the
//! window as a PNG, saved under `<workspace>/snapshots/` or copied.

use crate::app::App;
use crate::icons::ph;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Snap {
    Save,
    Copy,
    /// For the agent's `snapshot` tool: kept as a base64 PNG.
    Agent,
}

impl App {
    /// Asks the window for a screenshot; the chart area is cut from it
    /// when it arrives (next frame or so).
    pub fn take_snapshot(&mut self, kind: Snap) {
        self.snap = Some(kind);
        self.egui_ctx
            .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        self.egui_ctx.request_repaint();
    }

    pub(crate) fn poll_snapshot(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.snap else { return };
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else { return };
        self.snap = None;
        let ppp = ctx.pixels_per_point();
        let r = self.grid_rect;
        let [w, h] = image.size;
        let x0 = ((r.left() * ppp).round().max(0.0) as usize).min(w);
        let y0 = ((r.top() * ppp).round().max(0.0) as usize).min(h);
        let x1 = ((r.right() * ppp).round() as usize).clamp(x0, w);
        let y1 = ((r.bottom() * ppp).round() as usize).clamp(y0, h);
        let (cw, ch) = (x1 - x0, y1 - y0);
        if cw == 0 || ch == 0 {
            return;
        }
        let mut pixels = Vec::with_capacity(cw * ch);
        for y in y0..y1 {
            pixels.extend_from_slice(&image.pixels[y * w + x0..y * w + x1]);
        }
        let crop = egui::ColorImage::new([cw, ch], pixels);
        match kind {
            Snap::Agent => {
                // One image pixel per point keeps the picture (and its
                // tokens) small; it's still sharp enough to read.
                let img = downscale(&crop, ppp.round().max(1.0) as usize);
                self.claude.picture = Some(png_bytes(&img).map(|b| base64_of(&b)).unwrap_or_default());
            }
            Snap::Copy => {
                ctx.copy_image(crop);
                self.notify(ph::CAMERA, "Snapshot copied", "Paste it anywhere.", None, true);
            }
            Snap::Save => match self.save_png(&crop) {
                Ok(path) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.notify(ph::CAMERA, "Snapshot saved", &name, Some(path), true);
                }
                Err(e) => self.notify(ph::CAMERA, "Couldn't save the snapshot", &e, None, true),
            },
        }
    }

    fn save_png(&self, img: &egui::ColorImage) -> Result<std::path::PathBuf, String> {
        let dir = self.store.dir().join("snapshots");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let now = jiff::Zoned::now();
        let name = format!(
            "{}_{}_{}.png",
            self.chart.symbol.ticker().replace(['/', ':', ' '], "-"),
            self.chart.tf.label(),
            now.strftime("%Y-%m-%d_%H-%M-%S")
        );
        let path = dir.join(name);
        std::fs::write(&path, png_bytes(img)?).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

fn base64_of(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Every `k`th pixel, averaged over its k×k block.
fn downscale(img: &egui::ColorImage, k: usize) -> egui::ColorImage {
    if k <= 1 {
        return img.clone();
    }
    let [w, h] = img.size;
    let (nw, nh) = (w / k, h / k);
    let mut out = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            let mut sum = [0u32; 4];
            for dy in 0..k {
                for dx in 0..k {
                    let c = img.pixels[(y * k + dy) * w + x * k + dx].to_array();
                    for i in 0..4 {
                        sum[i] += c[i] as u32;
                    }
                }
            }
            let n = (k * k) as u32;
            out.push(egui::Color32::from_rgba_premultiplied(
                (sum[0] / n) as u8,
                (sum[1] / n) as u8,
                (sum[2] / n) as u8,
                (sum[3] / n) as u8,
            ));
        }
    }
    egui::ColorImage::new([nw, nh], out)
}

/// An image as PNG bytes.
fn png_bytes(img: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    let [w, h] = img.size;
    {
        let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        let data: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        writer.write_image_data(&data).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}
