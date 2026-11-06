use crate::text_appearance::BundledFont;
use crate::{PdfMasterError, Result};
use parley::{
    FontContext,
    fontique::{Blob, Collection, CollectionOptions},
};
pub(crate) struct EmbeddedFont {
    pub kind: BundledFont,
    pub blob: Blob<u8>,
}
pub(crate) struct EmbeddedFonts {
    pub fonts: Vec<EmbeddedFont>,
}
pub(crate) fn embedded_fonts() -> &'static EmbeddedFonts {
    static FONTS: std::sync::OnceLock<EmbeddedFonts> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| EmbeddedFonts {
        fonts: [
            (
                BundledFont::Sans,
                include_bytes!("../assets/fonts/NotoSans-Regular.otf").as_slice(),
            ),
            (
                BundledFont::CjkSc,
                include_bytes!("../assets/fonts/NotoSansCJKsc-Regular.otf").as_slice(),
            ),
            (
                BundledFont::Arabic,
                include_bytes!("../assets/fonts/NotoSansArabic-Regular.otf").as_slice(),
            ),
            (
                BundledFont::Devanagari,
                include_bytes!("../assets/fonts/NotoSansDevanagari-Regular.otf").as_slice(),
            ),
        ]
        .into_iter()
        .map(|(kind, data)| EmbeddedFont {
            kind,
            blob: Blob::new(std::sync::Arc::new(data)),
        })
        .collect(),
    })
}
pub(crate) fn family(kind: BundledFont) -> &'static str {
    match kind {
        BundledFont::Sans => "Noto Sans",
        BundledFont::CjkSc => "Noto Sans CJK SC",
        BundledFont::Arabic => "Noto Sans Arabic",
        BundledFont::Devanagari => "Noto Sans Devanagari",
    }
}
pub(crate) fn kind_for_blob(id: u64, index: u32) -> Result<BundledFont> {
    embedded_fonts()
        .fonts
        .iter()
        .find(|font| font.blob.id() == id && index == 0)
        .map(|font| font.kind)
        .ok_or_else(|| PdfMasterError::Serialization("unregistered font".into()))
}
pub(crate) fn font_context() -> Result<FontContext> {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    for font in &embedded_fonts().fonts {
        if collection.register_fonts(font.blob.clone(), None).len() != 1 {
            return Err(PdfMasterError::Serialization(
                "embedded font registration".into(),
            ));
        }
    }
    Ok(FontContext {
        collection,
        source_cache: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_only_the_four_embedded_fonts_with_stable_identities() {
        let mut context = font_context().unwrap();
        let names: Vec<_> = context.collection.family_names().collect();
        assert_eq!(names.len(), 4);
        for embedded in &embedded_fonts().fonts {
            assert!(names.contains(&family(embedded.kind)));
            assert_eq!(kind_for_blob(embedded.blob.id(), 0).unwrap(), embedded.kind);
        }
        assert!(kind_for_blob(u64::MAX, 0).is_err());
    }
}
