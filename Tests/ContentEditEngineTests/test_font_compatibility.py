import importlib.util
import io
import pathlib
import sys
import unittest

ROOT=pathlib.Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('compat',ROOT/'Resources/ContentEdit/font_compatibility.py')
compat=importlib.util.module_from_spec(spec);spec.loader.exec_module(compat)
fitz=compat.support().load_runtime()
compat.load_fonttools()
fixture_spec=importlib.util.spec_from_file_location('font_fixture',pathlib.Path(__file__).with_name('font_fixture.py'))
fixture=importlib.util.module_from_spec(fixture_spec);fixture_spec.loader.exec_module(fixture)
make_fixture=fixture.make_fixture

class FontCompatibilityTests(unittest.TestCase):
    def test_repairs_only_font_resources_and_preserves_text_content_and_metadata(self):
        raw=make_fixture(fitz)
        with fitz.open(stream=raw,filetype='pdf') as doc:
            doc.set_metadata({'title':'Synthetic local test'})
            streams=[doc.xref_stream(x) for x in doc[0].get_contents()]
            before=[(p.get_text(),p.rect) for p in doc]
            result,count=compat.normalize(doc)
        self.assertEqual(count,1)
        with fitz.open(stream=result,filetype='pdf') as doc:
            self.assertEqual(before,[(p.get_text(),p.rect) for p in doc])
            self.assertEqual(streams,[doc.xref_stream(x) for x in doc[0].get_contents()])
            self.assertEqual(doc.metadata['title'],'Synthetic local test')
            self.assertEqual(compat.normalize(doc),(None,0))
            font=doc[0].get_fonts()[0][0]
            child=int(doc.xref_get_key(font,'DescendantFonts')[1].strip('[] ').split()[0])
            self.assertEqual(doc.xref_get_key(child,'CIDToGIDMap'),('name','/Identity'))
    def test_normal_truetype_is_byte_preserving_noop(self):
        raw=make_fixture(fitz,valid=True)
        with fitz.open(stream=raw,filetype='pdf') as doc:
            self.assertEqual(compat.normalize(doc),(None,0))
    def test_encrypted_source_refused(self):
        with fitz.open(stream=make_fixture(fitz),filetype='pdf') as doc:
            data=doc.tobytes(encryption=fitz.PDF_ENCRYPT_AES_256,owner_pw='owner',user_pw='user')
        with fitz.open(stream=data,filetype='pdf') as doc:
            with self.assertRaisesRegex(ValueError,'Encrypted'):compat.normalize(doc)
    def test_signature_refused(self):
        with fitz.open(stream=make_fixture(fitz),filetype='pdf') as doc:
            x=doc.get_new_xref();doc.update_object(x,'<< /Type /Sig /ByteRange [0 10 20 30] >>')
            with self.assertRaisesRegex(ValueError,'Signature'):compat.normalize(doc)
    def test_mapping_ambiguity_refused(self):
        with fitz.open(stream=make_fixture(fitz),filetype='pdf') as doc:
            for x in range(1,doc.xref_length()):
                if doc.xref_get_key(x,'Subtype')==('name','/CIDFontType2'):
                    mapping=compat.ref(doc,x,'CIDToGIDMap')
                    doc.update_stream(mapping,bytes.fromhex('0000000b00c8'))
            with self.assertRaisesRegex(ValueError,'safely'):compat.normalize(doc)

if __name__=='__main__':unittest.main()
