use super::*;

/// Actions stay outside a single bounded scrolling body. Rectangles also let
/// layout tests check actual action accessibility at small sizes and scales.
pub(super) struct DialogResult {
    pub answer: Option<bool>,
    #[allow(dead_code)]
    pub cancel: egui::Rect,
    #[allow(dead_code)]
    pub proceed: egui::Rect,
    #[allow(dead_code)]
    pub body: egui::Rect,
    #[allow(dead_code)]
    pub scroll_offset: f32,
}
pub(super) fn show(
    ctx: &egui::Context,
    review: &MappingReview,
    focus: &mut bool,
    batch: bool,
) -> DialogResult {
    let viewport = ctx.viewport_rect();
    let mut result = DialogResult {
        answer: None,
        cancel: egui::Rect::NOTHING,
        proceed: egui::Rect::NOTHING,
        body: egui::Rect::NOTHING,
        scroll_offset: 0.0,
    };
    egui::Modal::new(egui::Id::unique("generate-nup-map")).show(ctx,|ui| {
        ui.set_width((viewport.width()-48.0).clamp(160.0,500.0));
        let header_top=ui.cursor().top();
        ui.add(egui::Label::new(egui::RichText::new(file_name(&review.source)).heading()).truncate()).on_hover_text(text(&review.source));
        if batch {ui.label("Review applies only to this file. Other files require their own review.");}
        let header_height=ui.cursor().top()-header_top;
        let footer=ui.spacing().interact_size.y.max(ui.text_style_height(&egui::TextStyle::Button)+2.0*ui.spacing().button_padding.y)+3.0*ui.spacing().item_spacing.y+32.0;
        let body_height=(viewport.height()-header_height-footer-32.0).max(16.0);
        let scroll=egui::ScrollArea::vertical().id_salt("nup-review-body").min_scrolled_height(16.0).max_height(body_height).show(ui,|ui| {
            if !review.preflight.violations.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color,"This input does not conform to the NUP v1 specification.");
                for violation in &review.preflight.violations {ui.label(violation);}
                ui.label("Compatibility preserves supported contents in NX; it does not repair NUP conformance. Consent cannot override unsupported content, identity conflicts, missing required content or integrity failures.");
            } else {ui.label("No representational NUP v1 violations detected. Content and identity checks still apply.");}
            if !review.preflight.unresolved_requirements.is_empty() {
                ui.colored_label(ui.visuals().warn_fg_color,"Unresolved native runtime requirements:");
                for warning in &review.preflight.unresolved_requirements {ui.label(warning);}
                ui.label("Continuing preserves these requirements as unresolved. It does not establish a satisfying application version or runtime compatibility.");
            }
            ui.label("Native launch metadata can select a nonzero program index; otherwise generation uses the documented index-zero convention. Missing or conflicting evidence stops generation.");
            ui.label("Continuing declares that stored program trees are complete. Metadata cannot prove this or guarantee the game will boot. Sources stay unchanged.");
            if review.mapping.is_none() {ui.label("The generated mapping is managed automatically for this session; there is no file to save.");}
        });
        result.body=scroll.inner_rect;
        result.scroll_offset=scroll.state.clamped_offset().y;
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let cancel=ui.button(if batch {"Cancel batch"} else {"Cancel"});
            result.cancel=cancel.rect;
            if *focus {cancel.request_focus();*focus=false;}
            let proceed=ui.button("Continue for this file");result.proceed=proceed.rect;
            if cancel.clicked() || ui.input(|i|i.key_pressed(egui::Key::Escape)) {result.answer=Some(false);}
            else if proceed.clicked() {result.answer=Some(true);}
        });
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn review() -> MappingReview {
        MappingReview{source:PathBuf::from(format!("{}.nup","Long source ".repeat(12))),mapping:None,preflight:nxformat_packer::import::NupPreflight{toc_sha256:[1;32],violations:(0..12).map(|i|format!("Package {i}: explicit source-format warning. Review the preserved contents and identity checks before continuing.")).collect(),unresolved_requirements:(0..8).map(|i|format!("DLC {i}: authoritative application version is unavailable; the minimum remains unresolved. Runtime compatibility is unverified.")).collect()}}
    }
    fn input(size: egui::Vec2, time: f64, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            events,
            ..Default::default()
        }
    }
    fn render_frame(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&egui::Context)) {
        ctx.run_pass(input, draw).drop_without_applying_deltas();
    }
    #[test]
    fn actions_fit_small_viewports_at_multiple_dpi_and_body_scrolls() {
        for scale in [1.0, 1.5, 2.0] {
            for physical_height in [400.0, 560.0, 680.0] {
                let ctx = egui::Context::default();
                theme::setup(&ctx);
                ctx.set_pixels_per_point(scale);
                let size = egui::vec2(900.0 / scale, physical_height / scale);
                let mut focus = true;
                let mut layout = None;
                for frame in 0..4 {
                    let _ = render_frame(&ctx, input(size, frame as f64 * 0.1, vec![]), |ctx| {
                        layout = Some(show(ctx, &review(), &mut focus, true))
                    });
                }
                let layout = layout.unwrap();
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                assert!(
                    viewport.contains_rect(layout.cancel),
                    "Cancel clipped: scale={scale} height={physical_height} {:?}",
                    layout.cancel
                );
                assert!(
                    viewport.contains_rect(layout.proceed),
                    "Continue clipped: scale={scale} height={physical_height} {:?}",
                    layout.proceed
                );
                assert!(layout.body.max.y <= layout.cancel.min.y);
                let center = layout.body.center();
                let mut offset = 0.0;
                for frame in 4..8 {
                    let events = if frame == 4 {
                        vec![
                            egui::Event::PointerMoved(center),
                            egui::Event::MouseWheel {
                                unit: egui::MouseWheelUnit::Point,
                                phase: egui::TouchPhase::Move,
                                source: egui::MouseWheelSource::Unknown,
                                delta: egui::vec2(0.0, -2000.0),
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    } else {
                        vec![]
                    };
                    let _ = render_frame(&ctx, input(size, frame as f64 * 0.1, events), |ctx| {
                        offset = show(ctx, &review(), &mut focus, true).scroll_offset
                    });
                }
                assert!(offset > 0.0, "warning body must remain scrollable");
            }
        }
    }
    #[test]
    fn escape_cancels_without_activating_mapping() {
        let ctx = egui::Context::default();
        let mut focus = true;
        let size = egui::vec2(600.0, 360.0);
        let _ = render_frame(&ctx, input(size, 0.0, vec![]), |ctx| {
            show(ctx, &review(), &mut focus, false);
        });
        let mut answer = None;
        let _ = render_frame(
            &ctx,
            input(
                size,
                0.1,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ),
            |ctx| answer = show(ctx, &review(), &mut focus, false).answer,
        );
        assert_eq!(answer, Some(false));
    }
}
