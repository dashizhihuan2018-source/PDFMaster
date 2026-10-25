#!/usr/bin/env python3
"""Local-only compatibility adapter; external runtimes are NOT redistributed.

Some producers put raw CID CFF in FontFile2/CIDFontType2 and use the
CIDToGIDMap as a code->CFF-CID map. PDFKit interprets it as TrueType GIDs.
Convert only this positively identified case, preserving glyph order, page
streams, encodings, ToUnicode, widths and all unrelated PDF objects.
"""
import argparse
import importlib.util
import io
import json
import math
import os
import pathlib
import stat
import sys

def support():
    path = pathlib.Path(__file__).with_name('engine.py')
    spec = importlib.util.spec_from_file_location('pdfmaster_content_support', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

def load_fonttools():
    root = pathlib.Path.home() / 'Library/Python/3.12/lib/python/site-packages/fontTools'
    if not sys.flags.isolated or not sys.flags.no_site:
        raise ValueError('Font compatibility requires isolated Python')
    for item in [root, *root.rglob('*')]:
        info = item.lstat()
        if stat.S_ISLNK(info.st_mode) or info.st_uid not in (os.getuid(), 0) or info.st_mode & 0o022:
            raise ValueError('Untrusted external fontTools runtime')
    spec = importlib.util.spec_from_file_location('fontTools', root / '__init__.py', submodule_search_locations=[str(root)])
    module = importlib.util.module_from_spec(spec); sys.modules['fontTools'] = module
    spec.loader.exec_module(module)
    if module.__version__ != '4.63.0':
        raise ValueError('External fontTools must be tested version 4.63.0')

def ref(doc, xref, key):
    kind, value = doc.xref_get_key(xref, key)
    return int(value.split()[0]) if kind == 'xref' else None

def normalize(doc):
    if doc.is_encrypted or doc.needs_pass:
        raise ValueError('Encrypted PDFs are not normalized')
    if doc.xref_length() > 100000 or doc.page_count > 10000:
        raise ValueError('PDF complexity limit')
    # Never invalidate signatures, including unsigned signature fields.
    for x in range(1,doc.xref_length()):
        if (doc.xref_get_key(x,'Type') == ('name','/Sig') or
            doc.xref_get_key(x,'FT') == ('name','/Sig') or
            doc.xref_get_key(x,'ByteRange')[0] != 'null'):
            raise ValueError('Signature-bearing PDFs are not normalized')
    targets = []
    for x in range(1,doc.xref_length()):
        if doc.xref_get_key(x,'Subtype') != ('name','/CIDFontType2'): continue
        descriptor = ref(doc,x,'FontDescriptor')
        program = ref(doc,descriptor,'FontFile2') if descriptor else None
        if not program: continue
        data = doc.xref_stream(program)
        if len(data) < 4 or data[:2] != b'\x01\x00': continue
        if len(data) > 16*1024*1024: raise ValueError('Font size limit')
        mapping = ref(doc,x,'CIDToGIDMap')
        if not mapping: raise ValueError('Unsupported malformed CFF mapping')
        targets.append((x,descriptor,program,data,doc.xref_stream(mapping)))
    if not targets: return None,0
    load_fonttools()
    from fontTools.cffLib import CFFFontSet
    from fontTools.fontBuilder import FontBuilder
    from fontTools.pens.ttGlyphPen import TTGlyphPen
    from fontTools.pens.cu2quPen import Cu2QuPen
    for x,descriptor,program,data,mapping in targets:
        cff=CFFFontSet();cff.decompile(io.BytesIO(data),None)
        if len(cff.topDictIndex) != 1: raise ValueError('Unsupported CFF font set')
        top=cff.topDictIndex[0];order=top.charset
        if not getattr(top,'ROS',None) or not 1 < len(order) <= 65535:
            raise ValueError('Unsupported CFF charset')
        if len(mapping)%2 or len(mapping)>131072:
            raise ValueError('Invalid CFF mapping size')
        # Positive identification: old codes are the CFF glyph indices and
        # the supposedly GID-valued entries are actually CFF internal CIDs.
        for cid in range(len(mapping)//2):
            value=int.from_bytes(mapping[2*cid:2*cid+2],'big')
            if value and (cid>=len(order) or order[cid] != 'cid%05d'%value):
                raise ValueError('CFF mapping cannot be normalized safely')
        matrix=getattr(top,'FontMatrix',[0.001,0,0,0.001,0,0])
        if matrix != [0.001,0,0,0.001,0,0]: raise ValueError('Unsupported CFF font matrix')
        fb=FontBuilder(1000,isTTF=True);fb.setupGlyphOrder(order);fb.setupCharacterMap({})
        glyphs={};metrics={}
        for name in order:
            pen=TTGlyphPen(None);cs=top.CharStrings[name]
            # At 1000 units/em this is <0.01 px at common reading sizes.
            cs.draw(Cu2QuPen(pen,max_err=0.1,reverse_direction=True))
            glyph=pen.glyph();glyph.recalcBounds(None)
            width=cs.width
            if not math.isfinite(width) or not 0 <= width <= 65535:
                raise ValueError('Invalid CFF width')
            glyphs[name]=glyph;metrics[name]=(round(width),getattr(glyph,'xMin',0))
        fb.setupGlyf(glyphs);fb.setupHorizontalMetrics(metrics)
        fb.setupHorizontalHeader(ascent=880,descent=-120)
        name='PDFMasterCompat'+str(x)
        fb.setupNameTable({'familyName':name,'styleName':'Regular','psName':name})
        fb.setupOS2();fb.setupPost();fb.setupMaxp()
        out=io.BytesIO();fb.save(out)
        # A shared descriptor/program might also serve another descendant.
        # Clone it per target, never modify shared source resources in place.
        new_program=doc.get_new_xref();doc.update_object(new_program,'<< >>')
        doc.update_stream(new_program,out.getvalue())
        doc.xref_set_key(new_program,'Length1',str(len(out.getvalue())))
        new_descriptor=doc.get_new_xref();doc.update_object(new_descriptor,doc.xref_object(descriptor))
        doc.xref_set_key(new_descriptor,'FontFile2','%d 0 R'%new_program)
        doc.xref_set_key(x,'FontDescriptor','%d 0 R'%new_descriptor)
        doc.xref_set_key(x,'CIDToGIDMap','/Identity')
    result=doc.tobytes(garbage=0,deflate=True)
    if len(result)>256*1024*1024: raise ValueError('Output size limit')
    return result,len(targets)

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--input',required=True);parser.add_argument('--output',required=True)
    args=parser.parse_args();helper=support()
    try:
        source=pathlib.Path(args.input)
        output=helper.validate_output_path(args.output,{source.resolve()})
        fitz=helper.load_runtime();raw=helper.read_regular(str(source),256*1024*1024)
        with fitz.open(stream=raw,filetype='pdf') as doc:
            count=doc.page_count;data,repaired=normalize(doc)
        if data is not None:
            with fitz.open(stream=data,filetype='pdf') as check:
                if check.page_count!=count or check.is_encrypted: raise ValueError('Invalid normalized PDF')
            helper.publish_exclusive(output,data)
        print(json.dumps({'repairedFonts':repaired}));return 0
    except Exception as error:
        print('Font compatibility refused: '+str(error),file=sys.stderr);return 1

if __name__=='__main__': sys.exit(main())
