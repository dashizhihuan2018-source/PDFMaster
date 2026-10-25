"""Self-authored glyphs only; never copies user's document or font programs."""
import io

def make_fixture(fitz, valid=False):
    from fontTools.fontBuilder import FontBuilder
    from fontTools.pens.t2CharStringPen import T2CharStringPen
    from fontTools.pens.ttGlyphPen import TTGlyphPen
    from fontTools.cffLib import FDArrayIndex, FDSelect, FontDict
    order = ['.notdef', 'cid00010', 'cid00200']
    glyphs = {}
    for i, name in enumerate(order):
        pen = TTGlyphPen(None) if valid else T2CharStringPen(1000, None)
        if i:
            # Distinct custom outlines, not a third-party font.
            pen.moveTo((0, 0)); pen.lineTo((700, 0)); pen.lineTo((700, 700))
            if i == 2: pen.lineTo((350,350))
            pen.lineTo((0,700)); pen.closePath()
        glyphs[name] = pen.glyph() if valid else pen.getCharString()
    fb = FontBuilder(1000, isTTF=valid)
    fb.setupGlyphOrder(order); fb.setupCharacterMap({65:order[1],66:order[2]})
    fb.setupHorizontalMetrics({n:(1000,0) for n in order})
    fb.setupHorizontalHeader(ascent=880,descent=-120)
    fb.setupNameTable({'familyName':'PDFMasterSynthetic','styleName':'Regular','psName':'PDFMasterSynthetic'})
    if valid:
        fb.setupGlyf(glyphs)
    else:
        fb.setupCFF('PDFMasterSynthetic',{},glyphs,{})
        top = fb.font['CFF '].cff.topDictIndex[0]
        top.ROS=('Adobe','Identity',0); top.CIDCount=201
        fd=FontDict();fd.Private=top.Private;fd.FontName='PDFMasterSynthetic'
        top.FDArray=FDArrayIndex();top.FDArray.append(fd)
        top.FDSelect=FDSelect(format=0);top.FDSelect.gidArray=[0]*len(order)
        del top.Private
    fb.setupOS2();fb.setupPost();fb.setupMaxp()
    out=io.BytesIO()
    if valid: fb.save(out)
    else: fb.font['CFF '].cff.compile(out,fb.font)
    d=fitz.open();page=d.new_page(width=300,height=200)
    def obj(value):
        x=d.get_new_xref();d.update_object(x,value);return x
    def stream(value):
        x=obj('<< >>');d.update_stream(x,value);return x
    program=stream(out.getvalue())
    desc=obj('<< /Type /FontDescriptor /FontName /PDFMasterSynthetic /Flags 32 /FontBBox [0 0 700 700] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 /FontFile2 %d 0 R >>'%program)
    mapping=stream(bytes.fromhex('0000000a00c8'))
    child=obj('<< /Type /Font /Subtype /CIDFontType2 /BaseFont /PDFMasterSynthetic /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 /FontDescriptor %d 0 R /CIDToGIDMap %s >>'%(desc, '/Identity' if valid else '%d 0 R'%mapping))
    unicode=stream(b'/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /TestUCS def /CMapType 2 def 1 begincodespacerange <0000> <ffff> endcodespacerange 2 beginbfchar <0001> <0041> <0002> <0042> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end')
    font=obj('<< /Type /Font /Subtype /Type0 /BaseFont /PDFMasterSynthetic /Encoding /Identity-H /DescendantFonts [%d 0 R] /ToUnicode %d 0 R >>'%(child,unicode))
    content=stream(b'BT /F1 70 Tf 30 70 Td <00010002> Tj ET')
    d.xref_set_key(page.xref,'Resources','<< /Font << /F1 %d 0 R >> >>'%font)
    d.xref_set_key(page.xref,'Contents','%d 0 R'%content)
    return d.tobytes()

if __name__ == '__main__':
    import importlib.util, pathlib, sys
    root=pathlib.Path(__file__).resolve().parents[2]
    spec=importlib.util.spec_from_file_location('compat',root/'Resources/ContentEdit/font_compatibility.py')
    compat=importlib.util.module_from_spec(spec);spec.loader.exec_module(compat)
    fitz=compat.support().load_runtime();compat.load_fonttools()
    output=pathlib.Path(sys.argv[1])
    (output/'broken.pdf').write_bytes(make_fixture(fitz))
    (output/'reference.pdf').write_bytes(make_fixture(fitz,True))
