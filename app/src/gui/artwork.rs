use eframe::egui;
use image::{DynamicImage, GenericImageView, ImageDecoder};
use std::{
    io::{Cursor, Read},
    path::Path,
};

pub const DEFAULT_SIZE: u32 = 256;
const OUTPUT_SIZES: [u32; 4] = [DEFAULT_SIZE, 512, 1024, 2048];
const MAX_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 16 * 1024 * 1024;

pub struct Artwork {
    pub image: DynamicImage,
    pub original: [u32; 2],
    pub texture: Option<egui::TextureHandle>,
}

impl Artwork {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_SOURCE_BYTES {
            return Err("Choose an image smaller than 128 MB.".into());
        }
        file.take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::decode(&bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            return Err("Choose an image smaller than 128 MB.".into());
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        if !matches!(
            reader.format(),
            Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
        ) {
            return Err("Choose a JPEG or PNG image.".into());
        }
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
        image.apply_orientation(orientation);
        let original = [image.width(), image.height()];
        if original.contains(&0) {
            return Err("The image is empty.".into());
        }
        Ok(Self {
            image,
            original,
            texture: None,
        })
    }
    pub fn texture(&mut self, ctx: &egui::Context) -> &egui::TextureHandle {
        self.texture.get_or_insert_with(|| {
            let rgba = self.image.thumbnail(1024, 1024).to_rgba8();
            ctx.load_texture(
                "artwork",
                egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    rgba.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )
        })
    }
    pub fn crop(
        &self,
        center: [f32; 2],
        zoom: f32,
        size: u32,
    ) -> Result<(Vec<u8>, egui::ColorImage), String> {
        if !OUTPUT_SIZES.contains(&size) {
            return Err("Choose a square size of 256, 512, 1024, or 2048 px.".into());
        }
        let (w, h) = self.image.dimensions();
        let side = ((w.min(h) as f32 / zoom.clamp(1.0, 4.0)).round() as u32).max(1);
        let x = (center[0] * w as f32 - side as f32 / 2.0)
            .round()
            .clamp(0.0, (w - side) as f32) as u32;
        let y = (center[1] * h as f32 - side as f32 / 2.0)
            .round()
            .clamp(0.0, (h - side) as f32) as u32;
        let cropped = self
            .image
            .crop_imm(x, y, side, side)
            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgb8();
        let mut bytes = Vec::new();
        for quality in [92, 80, 65, 50, 35] {
            bytes.clear();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality)
                .encode_image(&cropped)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 <= MAX_ARCHIVE_BYTES {
                let preview = image::imageops::thumbnail(&cropped, 256, 256);
                return Ok((
                    bytes,
                    egui::ColorImage::from_rgb(
                        [preview.width() as usize, preview.height() as usize],
                        preview.as_raw(),
                    ),
                ));
            }
        }
        Err("The artwork exceeds the 16 MB archive limit. Choose a smaller size.".into())
    }
}

pub struct Crop {
    pub key: String,
    pub art: Artwork,
    pub center: [f32; 2],
    pub zoom: f32,
    pub size: u32,
}
impl Crop {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("crop-artwork")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("Frame your artwork");
            ui.label(super::theme::muted(
                "Drag to position. Zoom to fill the square.",
            ));
            let texture = self.art.texture(ctx).id();
            let w = self.art.image.width() as f32;
            let h = self.art.image.height() as f32;
            let side = w.min(h) / self.zoom;
            self.center[0] = self.center[0].clamp(side / w / 2.0, 1.0 - side / w / 2.0);
            self.center[1] = self.center[1].clamp(side / h / 2.0, 1.0 - side / h / 2.0);
            ui.vertical_centered(|ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(304.0, 304.0), egui::Sense::drag());
                if response.dragged() {
                    self.center[0] -= response.drag_delta().x / rect.width() * side / w;
                    self.center[1] -= response.drag_delta().y / rect.height() * side / h;
                }
                let half = egui::vec2(side / w / 2.0, side / h / 2.0);
                let center = egui::pos2(self.center[0], self.center[1]);
                ui.painter().image(
                    texture,
                    rect,
                    egui::Rect::from_min_max(center - half, center + half),
                    egui::Color32::WHITE,
                );
                let stroke = egui::Stroke::new(0.5, egui::Color32::from_white_alpha(75));
                for t in [1.0 / 3.0, 2.0 / 3.0] {
                    ui.painter().line_segment(
                        [
                            rect.lerp_inside(egui::vec2(t, 0.0)),
                            rect.lerp_inside(egui::vec2(t, 1.0)),
                        ],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [
                            rect.lerp_inside(egui::vec2(0.0, t)),
                            rect.lerp_inside(egui::vec2(1.0, t)),
                        ],
                        stroke,
                    );
                }
            });
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=4.0).text("Zoom"));
            ui.horizontal(|ui| {
                ui.label("Output size");
                let size_label = |size| {
                    if size == DEFAULT_SIZE {
                        format!("{size} × {size} (Switch 1 default)")
                    } else {
                        format!("{size} × {size}")
                    }
                };
                egui::ComboBox::from_id_salt("artwork-output-size")
                    .selected_text(size_label(self.size))
                    .show_ui(ui, |ui| {
                        for size in OUTPUT_SIZES {
                            ui.selectable_value(&mut self.size, size, size_label(size));
                        }
                    });
            });
            ui.label(super::theme::muted(format!(
                "{} × {} source · JPEG output",
                self.art.original[0], self.art.original[1]
            )));
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
                if super::theme::button(ui, super::theme::Icon::Check, "Use artwork", true)
                    .clicked()
                {
                    answer = Some(true);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_is_square_and_clamps_edges() {
        let art = Artwork {
            image: DynamicImage::ImageRgb8(image::RgbImage::from_fn(600, 300, |x, _| {
                if x < 300 {
                    image::Rgb([255, 0, 0])
                } else {
                    image::Rgb([0, 0, 255])
                }
            })),
            original: [600, 300],
            texture: None,
        };
        for (center, zoom) in [([0.0, 0.0], 1.0), ([1.0, 1.0], 4.0)] {
            let (data, _) = art.crop(center, zoom, DEFAULT_SIZE).unwrap();
            let decoded = image::load_from_memory(&data).unwrap();
            assert_eq!(decoded.dimensions(), (256, 256));
            let pixel = decoded.to_rgb8().get_pixel(128, 128).0;
            assert!(if center[0] == 0.0 {
                pixel[0] > 240 && pixel[2] < 10
            } else {
                pixel[2] > 240 && pixel[0] < 10
            });
        }
    }
    #[test]
    fn crops_multiple_output_sizes() {
        let art = Artwork {
            image: DynamicImage::new_rgb8(256, 256),
            original: [256, 256],
            texture: None,
        };
        for size in [512, 1024, 2048] {
            let (bytes, _) = art.crop([0.5, 0.5], 1.0, size).unwrap();
            let reader =
                image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
            assert_eq!(reader.into_dimensions().unwrap(), (size, size));
        }
        assert!(art.crop([0.5, 0.5], 1.0, 768).is_err());
    }
    #[test]
    fn keeps_source_resolution_for_cropping() {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2560, 8)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let art = Artwork::decode(encoded.get_ref()).unwrap();
        assert_eq!(art.image.dimensions(), (2560, 8));
    }
    #[test]
    fn rejects_unsupported_and_oversized_images() {
        assert!(Artwork::decode(b"GIF89a").is_err());
        assert!(Artwork::decode(&vec![0; MAX_SOURCE_BYTES as usize + 1]).is_err());
    }
}
