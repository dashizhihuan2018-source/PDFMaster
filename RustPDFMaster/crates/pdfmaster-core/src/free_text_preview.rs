use lopdf::Object;

use crate::{FreeTextAnnotation, PdfMasterError, Result, Session};

impl Session {
    pub fn render_free_text_background_png(
        &self,
        expected: &FreeTextAnnotation,
        scale: f32,
    ) -> Result<Vec<u8>> {
        self.selected_free_text(expected)?;
        self.preflight_render_page(expected.page_index)?;
        drop(self.render_png(expected.page_index, scale)?);
        let mut candidate = self.document.clone();
        let page_id = crate::document::page_object_ids(&candidate)?
            .get(expected.page_index)
            .copied()
            .ok_or(PdfMasterError::InvalidOperation(
                "FreeText page is unavailable",
            ))?;
        let mut annots = crate::notes::resolve(
            &candidate,
            candidate
                .get_object(page_id)
                .and_then(Object::as_dict)
                .and_then(|page| page.get(b"Annots"))
                .map_err(|error| PdfMasterError::Malformed(error.to_string()))?,
        )?
        .as_array()
        .map_err(|_| PdfMasterError::Malformed("page Annots must be an array".into()))?
        .clone();
        if expected.annotation_index >= annots.len() {
            return Err(PdfMasterError::InvalidOperation(
                "FreeText annotation index is unavailable",
            ));
        }
        annots.remove(expected.annotation_index);
        let annots_id = candidate.add_object(annots);
        candidate
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| PdfMasterError::Malformed(error.to_string()))?
            .set("Annots", annots_id);
        let bytes = crate::document::serialize(&candidate)?;
        Session::open(&bytes)?.render_png(expected.page_index, scale)
    }
}
