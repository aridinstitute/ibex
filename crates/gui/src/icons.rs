//! Native SF Symbols on macOS, with embedded Lucide fallbacks elsewhere.

use gpui_kit::component::Icon;

pub fn symbol(name: &'static str, fallback: impl Into<Icon>) -> Icon {
    #[cfg(target_os = "macos")]
    if let Some(icon) = native::icon(name) {
        return icon;
    }
    let _ = name;
    fallback.into()
}

#[cfg(target_os = "macos")]
mod native {
    use std::{cell::RefCell, collections::HashMap};

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use gpui_kit::component::Icon;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSImageSymbolConfiguration,
    };
    use objc2_foundation::{NSDictionary, NSString};

    thread_local! {
        static CACHE: RefCell<HashMap<&'static str, Option<Icon>>> = RefCell::new(HashMap::new());
    }

    pub fn icon(name: &'static str) -> Option<Icon> {
        CACHE.with(|cache| {
            cache
                .borrow_mut()
                .entry(name)
                .or_insert_with(|| load_svg(name).map(|svg| Icon::default().data(&svg)))
                .clone()
        })
    }

    fn load_svg(name: &str) -> Option<Vec<u8>> {
        autoreleasepool(|_| {
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(name),
                None,
            )?;
            // Rasterize above Retina toolbar resolution. GPUI takes the SVG's
            // alpha mask, so native symbols inherit text color just like SVGs.
            let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(48., 0.);
            let image = image.imageWithSymbolConfiguration(&config)?;
            let tiff = image.TIFFRepresentation()?;
            let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)?;
            // SAFETY: an empty dictionary contains no incorrectly typed properties.
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?;
            let data = STANDARD.encode(png.to_vec());
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 48 48"><image width="48" height="48" preserveAspectRatio="xMidYMid meet" xlink:href="data:image/png;base64,{data}"/></svg>"#
            );
            Some(svg.into_bytes())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::{icon, load_svg};
        use gpui_kit::SvgRenderer;
        use std::sync::Arc;

        #[test]
        fn loads_system_symbols_and_rejects_unknown_names() {
            assert!(icon("book.closed").is_some());
            assert!(icon("line.3.horizontal.decrease").is_some());
            assert!(icon("sun.max").is_some());
            assert!(icon("moon").is_some());
            assert!(icon("oeuvre.nonexistent.symbol").is_none());
        }

        #[test]
        fn system_symbol_renders_visible_pixels_through_gpui() {
            let svg = load_svg("book.closed").expect("system symbol");
            let renderer = SvgRenderer::new(Arc::new(()));
            let image = renderer.render_single_frame(&svg, 1.).expect("valid SVG");
            let pixels = image.as_bytes(0).expect("first frame");
            let (pixels, _) = pixels.as_chunks::<4>();
            assert!(pixels.iter().any(|pixel| pixel[3] > 0));
            assert!(pixels.iter().any(|pixel| pixel[3] == 0));
        }
    }
}
